//! Browser-level end-to-end tests for the dashboard UI, driven by
//! chromiumoxide against a real headless Chromium.
//!
//! The server is booted in-process on an ephemeral port (same pattern as
//! `dashboard_it.rs`) and the tests exercise the page the way a user
//! would: load the index, find the config cards, click "Generate Config
//! from Profile" and wait for the preview panel.
//!
//! Requires a Chromium binary. Resolution order:
//!   1. `CAREERAI_CHROMIUM` env var pointing at the executable;
//!   2. `~/.cache/careerai/chromium/chrome-headless-shell-linux64/chrome-headless-shell`
//!      (what `scripts/fetch-chromium.sh` installs);
//!   3. common system locations (`chromium`, `google-chrome`, ...).
//!
//! When no binary is found the tests skip with a hint instead of
//! failing, so machines without a browser (and CI) stay green.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;

use careerai_dashboard::{run, ServeOptions};
use careerai_db::pool::pool_in_memory;

const CARD_SELECTOR: &str = ".config-gen-card";
const GENERATE_BUTTON: &str = ".config-gen-card button";
const PREVIEW_PANEL: &str = "#config-diff-panel";
const STATUS_MSG: &str = "#config-gen-status-msg";
const AFTER_PREVIEW: &str = "#config-after";

fn chromium_executable() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CAREERAI_CHROMIUM") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = vec![
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/usr/bin/chromium-browser"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/google-chrome-stable"),
        PathBuf::from("/snap/bin/chromium"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.insert(
            0,
            home.join(
                ".cache/careerai/chromium/chrome-headless-shell-linux64/chrome-headless-shell",
            ),
        );
    }
    candidates.into_iter().find(|p| p.is_file())
}

async fn pick_free_port() -> u16 {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

async fn wait_for_server(port: u16) {
    for i in 0..40 {
        match tokio::net::TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await
        {
            Ok(_) => return,
            Err(e) if i % 5 == 0 => eprintln!("connect attempt {i}: {e}"),
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("server never accepted a connection on port {port}");
}

async fn launch_browser(width: u32, height: u32) -> (Browser, tokio::task::JoinHandle<()>) {
    let exe = chromium_executable().expect("chromium resolved before launch_browser");
    let config = BrowserConfig::builder()
        .chrome_executable(&exe)
        .no_sandbox()
        .new_headless_mode()
        .window_size(width, height)
        .arg("--disable-dev-shm-usage")
        .launch_timeout(Duration::from_secs(45))
        .build()
        .expect("browser config");
    let (browser, mut handler) = Browser::launch(config).await.expect("launch chromium");
    // Drain the event stream for the browser's lifetime. Must NOT break
    // on `Err` events: newer Chrome/headless-shell emit CDP events that
    // chromiumoxide 0.7 does not model, and exiting the handler kills
    // every subsequent CDP command with ChannelSendError(Canceled).
    let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });
    (browser, handle)
}

/// Poll until `selector` is present in the DOM or the deadline passes.
async fn wait_for_selector(page: &Page, selector: &str, deadline: Instant) {
    loop {
        let present: bool = page
            .evaluate(format!("!!document.querySelector('{selector}')"))
            .await
            .is_ok_and(|r| r.into_value::<bool>().unwrap_or(false));
        if present {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "selector {selector} never appeared in the DOM"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Poll until `selector` has a rendered box (client rects), i.e. it is
/// visible and clickable, or the deadline passes.
async fn wait_for_visible(page: &Page, selector: &str, deadline: Instant) {
    loop {
        let visible: bool = page
            .evaluate(format!(
                "document.querySelector('{selector}')?.getClientRects().length > 0"
            ))
            .await
            .is_ok_and(|r| r.into_value::<bool>().unwrap_or(false));
        if visible {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "selector {selector} never became visible"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn open_index(browser: &Browser, port: u16, deadline: Instant) -> Page {
    let page = browser
        .new_page(format!("http://127.0.0.1:{port}/"))
        .await
        .expect("open index page");
    wait_for_selector(&page, CARD_SELECTOR, deadline).await;
    page
}

/// The config cards live inside the "Config & Sources" tab panel, which
/// starts hidden behind the default "Pipeline & Funnel" tab. Click the
/// tab button the way a user would, then wait for the card to render.
async fn open_config_tab(page: &Page, deadline: Instant) {
    page.find_element("#tab-btn-config")
        .await
        .expect("find config tab button")
        .click()
        .await
        .expect("click config tab");
    wait_for_visible(page, CARD_SELECTOR, deadline).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generate_config_card_flows_preview_in_real_browser() {
    let Some(_exe) = chromium_executable() else {
        eprintln!(
            "SKIP: no chromium executable found — set CAREERAI_CHROMIUM or run scripts/fetch-chromium.sh"
        );
        return;
    };

    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool,
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;

    let (mut browser, _handler) = launch_browser(1280, 800).await;
    let deadline = Instant::now() + Duration::from_secs(15);
    let page = open_index(&browser, port, deadline).await;

    // The config cards live in a tab that starts hidden; a user reaches
    // them by clicking "Config & Sources".
    let initially_hidden: bool = page
        .evaluate(format!(
            "document.querySelector('{CARD_SELECTOR}').getClientRects().length === 0"
        ))
        .await
        .expect("eval initial visibility")
        .into_value::<bool>()
        .expect("initial visibility bool");
    assert!(
        initially_hidden,
        "config-gen-card should be hidden behind the default tab"
    );

    open_config_tab(&page, deadline).await;

    // Regression guard for the R6 layout change: the generate card must
    // sit OUTSIDE the LLM backend form.
    let nested: bool = page
        .evaluate("!!document.querySelector('#llm-config-form .config-gen-card')")
        .await
        .expect("eval nested check")
        .into_value::<bool>()
        .expect("nested bool");
    assert!(
        !nested,
        "config-gen-card must not be nested inside the LLM form"
    );

    let card_text = page
        .find_element(CARD_SELECTOR)
        .await
        .expect("find config-gen-card")
        .inner_text()
        .await
        .expect("card text")
        .unwrap_or_default();
    assert!(
        card_text.contains("Generate Config from Profile"),
        "card text: {card_text:?}"
    );

    // Click the generate button and wait for the preview panel to show.
    generate_config_preview(&page).await;

    browser.close().await.expect("close browser");
    server.abort();
}

/// Click "Generate Config from Profile" and wait for the preview panel
/// to become visible, then assert the status message and that the
/// generated YAML preview is populated.
async fn generate_config_preview(page: &Page) {
    let deadline = Instant::now() + Duration::from_secs(15);
    page.find_element(GENERATE_BUTTON)
        .await
        .expect("find generate button")
        .click()
        .await
        .expect("click generate");

    loop {
        let display: String = page
            .evaluate(format!(
                "document.querySelector('{PREVIEW_PANEL}') ? document.querySelector('{PREVIEW_PANEL}').style.display : ''"
            ))
            .await
            .expect("eval panel display")
            .into_value::<String>()
            .unwrap_or_default();
        if display == "block" {
            break;
        }
        if Instant::now() >= deadline {
            let status = page
                .find_element(STATUS_MSG)
                .await
                .ok()
                .and_then(|el| futures::executor::block_on(el.inner_text()).ok().flatten())
                .unwrap_or_default();
            panic!("preview panel never became visible (display={display:?}, status={status:?})");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let status = page
        .find_element(STATUS_MSG)
        .await
        .expect("find status msg")
        .inner_text()
        .await
        .expect("status text")
        .unwrap_or_default();
    assert!(
        status.contains("Preview ready"),
        "status message after generate: {status:?}"
    );

    let after = page
        .find_element(AFTER_PREVIEW)
        .await
        .expect("find config-after")
        .inner_text()
        .await
        .expect("after text")
        .unwrap_or_default();
    assert!(
        !after.trim().is_empty(),
        "config-after preview must contain the generated yaml"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mobile_viewport_renders_config_cards_without_horizontal_overflow() {
    let Some(_exe) = chromium_executable() else {
        eprintln!(
            "SKIP: no chromium executable found — set CAREERAI_CHROMIUM or run scripts/fetch-chromium.sh"
        );
        return;
    };

    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool,
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;

    let (mut browser, _handler) = launch_browser(390, 844).await;
    let deadline = Instant::now() + Duration::from_secs(15);
    let page = open_index(&browser, port, deadline).await;

    open_config_tab(&page, deadline).await;

    let boxed = page
        .find_element(CARD_SELECTOR)
        .await
        .expect("find config-gen-card")
        .bounding_box()
        .await
        .expect("bounding box");
    assert!(
        boxed.width > 0.0,
        "config-gen-card has zero width on a 390px viewport"
    );

    let overflow: bool = page
        .evaluate("document.documentElement.scrollWidth > window.innerWidth")
        .await
        .expect("eval overflow")
        .into_value::<bool>()
        .expect("overflow bool");
    if overflow {
        let offenders: String = page
            .evaluate(
                "(function(){ var out = []; document.querySelectorAll('*').forEach(function(el){ \
                 var r = el.getBoundingClientRect(); \
                 if (r.right > window.innerWidth + 1 && r.width > 0) { \
                 var c = el.className; \
                 if (typeof c !== 'string') { c = c.baseVal || ''; } \
                 out.push(el.tagName.toLowerCase() + (c ? '.' + String(c).split(' ')[0] : '') + ' right=' + Math.round(r.right)); \
                 } }); return out.slice(0, 10).join(' | '); })()",
            )
            .await
            .expect("eval offenders")
            .into_value::<String>()
            .expect("offenders string");
        panic!("page overflows horizontally on a 390px viewport; offenders: {offenders}");
    }

    browser.close().await.expect("close browser");
    server.abort();
}

async fn assert_all_tabs_fit_viewport(browser: &Browser, port: u16, width: u32) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let page = open_index(browser, port, deadline).await;
    let tabs = [
        "funnel", "events", "config", "actions", "explorer", "commands", "canvas", "chat",
    ];

    for tab in tabs {
        let button_selector = format!("#tab-btn-{tab}");
        let panel_selector = format!("#tab-{tab}");
        page.find_element(&button_selector)
            .await
            .unwrap_or_else(|_| panic!("find {button_selector}"))
            .click()
            .await
            .unwrap_or_else(|_| panic!("click {button_selector}"));
        wait_for_visible(&page, &panel_selector, deadline).await;

        if tab != "funnel" {
            let zone_selector = format!("{panel_selector} .mission-panel-head");
            wait_for_visible(&page, &zone_selector, deadline).await;
        }

        let brand_width: f64 = page
            .evaluate("document.querySelector('.brand-container').getBoundingClientRect().width")
            .await
            .expect("eval brand width")
            .into_value::<f64>()
            .expect("brand width number");
        assert!(
            brand_width >= 120.0,
            "brand collapsed to {brand_width}px on tab {tab} at {width}px"
        );

        let brand_text_is_visible: bool = page
            .evaluate(
                "(function(){ var b=document.querySelector('.brand'); \
                 return b.getBoundingClientRect().width >= 80 && b.scrollWidth <= b.clientWidth + 1; })()",
            )
            .await
            .expect("eval brand text visibility")
            .into_value::<bool>()
            .expect("brand text visibility bool");
        assert!(
            brand_text_is_visible,
            "brand text is clipped on tab {tab} at {width}px"
        );

        let active_tab_is_visible: bool = page
            .evaluate(format!(
                "(function(){{ var n=document.querySelector('.tab-nav').getBoundingClientRect(); \
                 var b=document.querySelector('{button_selector}').getBoundingClientRect(); \
                 return b.left >= n.left - 1 && b.right <= n.right + 1 && b.width > 40; }})()"
            ))
            .await
            .expect("eval active tab visibility")
            .into_value::<bool>()
            .expect("active tab visibility bool");
        assert!(
            active_tab_is_visible,
            "active tab {tab} is clipped inside the nav rail at {width}px"
        );

        let overflow: bool = page
            .evaluate("document.documentElement.scrollWidth > window.innerWidth + 1")
            .await
            .expect("eval cross-tab overflow")
            .into_value::<bool>()
            .expect("cross-tab overflow bool");
        if overflow {
            let offenders: String = page
                .evaluate(
                    "(function(){ var out = []; document.querySelectorAll('*').forEach(function(el){ \
                     var r = el.getBoundingClientRect(); \
                     if (r.right > window.innerWidth + 1 && r.width > 0) { \
                     var c = el.className; if (typeof c !== 'string') { c = c.baseVal || ''; } \
                     out.push(el.tagName.toLowerCase() + (c ? '.' + String(c).split(' ')[0] : '') + ' right=' + Math.round(r.right)); \
                     } }); return out.slice(0, 12).join(' | '); })()",
                )
                .await
                .expect("eval cross-tab offenders")
                .into_value::<String>()
                .expect("cross-tab offenders string");
            panic!("tab {tab} overflows at {width}px; offenders: {offenders}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_dashboard_tab_fits_desktop_and_mobile_viewports() {
    let Some(_exe) = chromium_executable() else {
        eprintln!(
            "SKIP: no chromium executable found — set CAREERAI_CHROMIUM or run scripts/fetch-chromium.sh"
        );
        return;
    };

    let pool = pool_in_memory().await.expect("in-memory pool");
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool,
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;

    for (width, height) in [(1440, 1000), (390, 844)] {
        let (mut browser, _handler) = launch_browser(width, height).await;
        assert_all_tabs_fit_viewport(&browser, port, width).await;
        browser.close().await.expect("close browser");
    }

    server.abort();
}
