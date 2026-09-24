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

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    // `BROWSER_SERIAL` is a std mutex held across `.await`s by design —
    // it serializes CPU-heavy Chromium instances on constrained CI
    // runners. Deliberate, documented, test-only.
    clippy::await_holding_lock
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tempfile::TempDir;

use careerai_dashboard::{run, ServeOptions};
use careerai_db::pool::pool_in_memory;

const CARD_SELECTOR: &str = ".config-gen-card";
const GENERATE_BUTTON: &str = ".config-gen-card button";
const PREVIEW_PANEL: &str = "#config-diff-panel";
const STATUS_MSG: &str = "#config-gen-status-msg";
const AFTER_PREVIEW: &str = "#config-after";

/// Headless Chromium is CPU-heavy; running several instances at once on a
/// constrained CI runner starves each other's renderer and produces
/// spurious empty-text/timeout failures. Serialize the browser tests in
/// this file so at most one Chromium instance runs at a time.
static BROWSER_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

async fn launch_browser(
    width: u32,
    height: u32,
) -> (Browser, tokio::task::JoinHandle<()>, TempDir) {
    let exe = chromium_executable().expect("chromium resolved before launch_browser");
    let profile_dir = tempfile::Builder::new()
        .prefix("careerai-dashboard-browser-")
        .tempdir()
        .expect("browser profile tempdir");
    let config = BrowserConfig::builder()
        .chrome_executable(&exe)
        .no_sandbox()
        .new_headless_mode()
        .window_size(width, height)
        .viewport(chromiumoxide::handler::viewport::Viewport {
            width,
            height,
            ..Default::default()
        })
        .arg("--disable-dev-shm-usage")
        .user_data_dir(profile_dir.path())
        .launch_timeout(Duration::from_secs(45))
        .build()
        .expect("browser config");
    let (browser, mut handler) = Browser::launch(config).await.expect("launch chromium");
    // Drain the event stream for the browser's lifetime. Must NOT break
    // on `Err` events: newer Chrome/headless-shell emit CDP events that
    // chromiumoxide 0.7 does not model, and exiting the handler kills
    // every subsequent CDP command with ChannelSendError(Canceled).
    let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });
    (browser, handle, profile_dir)
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
    let _serial = BROWSER_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    let (mut browser, _handler, _profile_dir) = launch_browser(1280, 800).await;
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

    let card_deadline = Instant::now() + Duration::from_secs(60);
    let mut card_text = String::new();
    while Instant::now() < card_deadline {
        let text: String = page
            .evaluate(format!(
                "document.querySelector('{CARD_SELECTOR}')?.innerText || ''"
            ))
            .await
            .ok()
            .and_then(|r| r.into_value::<String>().ok())
            .unwrap_or_default();
        if text.contains("Generate Config from Profile") {
            card_text = text;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
    let _serial = BROWSER_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    let (mut browser, _handler, _profile_dir) = launch_browser(390, 844).await;
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
        "funnel",
        "events",
        "config",
        "actions",
        "explorer",
        "commands",
        "analytics",
        "bounties",
        "chat",
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
    let _serial = BROWSER_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let (mut browser, _handler, _profile_dir) = launch_browser(width, height).await;
        assert_all_tabs_fit_viewport(&browser, port, width).await;
        browser.close().await.expect("close browser");
    }

    server.abort();
}

/// Exercise the workspace with enough rows to cross the idle-render boundary.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_saved_searches_palette_and_refresh_in_real_browser() {
    use chromiumoxide::page::ScreenshotParams;
    let _serial = BROWSER_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if chromium_executable().is_none() {
        eprintln!("SKIP: set CAREERAI_CHROMIUM for workspace browser coverage");
        return;
    }
    let pool = pool_in_memory().await.expect("database");
    seed_workspace_listings(&pool).await;
    let port = pick_free_port().await;
    let opts = ServeOptions {
        port,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        refresh_seconds: 0,
        pool: pool.clone(),
    };
    let server = tokio::spawn(async move {
        let _ = run(opts).await;
    });
    wait_for_server(port).await;
    let (mut browser, _handler, _profile_dir) = launch_browser(1440, 1100).await;
    let page = open_index(&browser, port, Instant::now() + Duration::from_secs(15)).await;
    wait_for_selector(
        &page,
        "#activity-chart li",
        Instant::now() + Duration::from_secs(10),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    if let Some(dir) = std::env::var_os("CAREERAI_SCREENSHOT_DIR") {
        page.save_screenshot(
            ScreenshotParams::builder().full_page(false).build(),
            PathBuf::from(dir).join("workspace-desktop.png"),
        )
        .await
        .expect("desktop screenshot");
    }
    test_desktop_explorer(&page, &pool).await;
    page.evaluate("switchTab('funnel'); window._toggleTheme();")
        .await
        .expect("dark theme");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    if let Some(dir) = std::env::var_os("CAREERAI_SCREENSHOT_DIR") {
        page.save_screenshot(
            ScreenshotParams::builder().full_page(false).build(),
            PathBuf::from(dir).join("workspace-dark.png"),
        )
        .await
        .expect("dark screenshot");
    }
    browser.close().await.expect("close desktop browser");
    test_mobile_view(port).await;
    server.abort();
}
#[allow(clippy::too_many_lines)]
async fn test_desktop_explorer(page: &chromiumoxide::Page, pool: &sqlx::SqlitePool) {
    page.find_element("#command-trigger")
        .await
        .expect("palette trigger")
        .click()
        .await
        .expect("open palette");
    let focused: bool = page.evaluate("document.querySelector('#command-palette').open && document.activeElement.id === 'command-search'").await.expect("focus check").into_value().expect("bool");
    assert!(focused, "palette must focus its search field");
    page.evaluate("document.querySelector('#command-search').value = 'opportunities'; document.querySelector('#command-search').dispatchEvent(new Event('input')); document.querySelector('#command-search').dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', bubbles:true}));").await.expect("keyboard navigation");
    wait_for_visible(
        page,
        "#explorer-search-input",
        Instant::now() + Duration::from_secs(5),
    )
    .await;
    let destination_focused: bool = page
        .evaluate("document.activeElement.id === 'tab-btn-explorer'")
        .await
        .expect("palette destination focus")
        .into_value()
        .expect("bool");
    assert!(
        destination_focused,
        "palette navigation must focus the destination"
    );

    page.evaluate("document.querySelector('#flt-remote').checked = true; filterExplorerTable();")
        .await
        .expect("filter remote");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let loaded: bool = page
            .evaluate("document.querySelectorAll('.explorer-row').length === 135")
            .await
            .expect("rows")
            .into_value()
            .expect("bool");
        if loaded {
            break;
        }
        assert!(Instant::now() < deadline, "idle chunks never completed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let visible_remote: bool = page.evaluate("[...document.querySelectorAll('.explorer-row')].filter(r => r.style.display !== 'none').every(r => r.dataset.remote === 'true')").await.expect("chunk filter").into_value().expect("bool");
    assert!(
        visible_remote,
        "every idle chunk must respect current filters"
    );

    page.find_element("#save-search-toggle")
        .await
        .expect("save search")
        .click()
        .await
        .expect("open save form");
    page.evaluate("document.querySelector('#saved-search-name').value = 'Remote roles'; document.querySelector('#save-search-form').requestSubmit();").await.expect("save filters");
    page.reload().await.expect("reload");
    wait_for_selector(
        page,
        ".saved-search button",
        Instant::now() + Duration::from_secs(10),
    )
    .await;
    page.find_element(".saved-search button")
        .await
        .expect("saved view")
        .click()
        .await
        .expect("restore filters");
    let restored: bool = page
        .evaluate("document.querySelector('#flt-remote').checked")
        .await
        .expect("restored filter")
        .into_value()
        .expect("bool");
    assert!(restored, "saved searches must persist across reloads");

    page.evaluate("window.originalFetch = window.fetch; window.fetch = function(url, options) { return url === '/api/v1/explorer' ? Promise.resolve({ok:false}) : window.originalFetch(url, options); }; refreshExplorerListings();").await.expect("simulate refresh failure");
    wait_for_visible(
        page,
        "#explorer-load-error",
        Instant::now() + Duration::from_secs(5),
    )
    .await;
    page.evaluate("filterExplorerTable();")
        .await
        .expect("filter after failure");
    let retry_visible: bool = page
        .evaluate(
            "document.querySelector('#explorer-load-error button').getClientRects().length > 0",
        )
        .await
        .expect("retry remains available")
        .into_value()
        .expect("bool");
    assert!(
        retry_visible,
        "filtering must preserve the refresh failure and retry"
    );

    page.evaluate("window.fetch = window.originalFetch; document.querySelector('#explorer-load-error button').click();").await.expect("retry with working network");
    sqlx::query("UPDATE listings SET state = 'filtered_out', score = 0 WHERE id = 'workspace-0'")
        .execute(pool)
        .await
        .expect("change existing listing");
    page.evaluate("refreshExplorerListings()")
        .await
        .expect("refresh");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let refreshed: bool = page.evaluate("[...document.querySelectorAll('.explorer-row')].some(r => r.textContent.includes('workspace-0') && r.dataset.state === 'filtered_out' && r.textContent.includes('0%'))").await.expect("refreshed state").into_value().expect("bool");
        if refreshed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "refresh must update existing rows and display zero scores"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn test_mobile_view(port: u16) {
    use chromiumoxide::page::ScreenshotParams;
    let (mut mobile, _mobile_handler, _profile_dir) = launch_browser(390, 844).await;
    let page = open_index(&mobile, port, Instant::now() + Duration::from_secs(15)).await;
    wait_for_selector(
        &page,
        "#activity-chart li",
        Instant::now() + Duration::from_secs(10),
    )
    .await;
    let dimensions: serde_json::Value = page
        .evaluate("({width:innerWidth, scroll:document.documentElement.scrollWidth})")
        .await
        .expect("mobile dimensions")
        .into_value()
        .expect("dimensions");
    assert!(
        dimensions["scroll"].as_u64() <= dimensions["width"].as_u64(),
        "mobile overflow: {dimensions}"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    if let Some(dir) = std::env::var_os("CAREERAI_SCREENSHOT_DIR") {
        page.save_screenshot(
            ScreenshotParams::builder().full_page(false).build(),
            PathBuf::from(dir).join("workspace-mobile.png"),
        )
        .await
        .expect("mobile screenshot");
    }
    mobile.close().await.expect("close mobile browser");
}
async fn seed_workspace_listings(pool: &sqlx::SqlitePool) {
    let companies = [
        "Northstar Robotics",
        "Linear Labs",
        "Orbit Systems",
        "Fieldwork",
        "Forma",
        "Atlas Engineering",
    ];
    let titles = [
        "Senior Rust Engineer",
        "AI Platform Engineer",
        "Embedded Software Engineer",
        "Backend Engineer",
        "Robotics Engineer",
        "Software Engineer",
    ];
    for index in 0..135 {
        let state = if index < 12 {
            "shortlisted"
        } else if index < 18 {
            "submitted"
        } else {
            "discovered"
        };
        sqlx::query("INSERT INTO listings (id, source, external_id, title, company, location, url, description, state, score) VALUES (?, 'greenhouse', ?, ?, ?, ?, 'https://example.com/job', 'Build useful software with Rust and Python.', ?, ?)")
            .bind(format!("workspace-{index}"))
            .bind(index.to_string())
            .bind(titles[index % titles.len()])
            .bind(companies[index % companies.len()])
            .bind(if index % 2 == 0 { "Remote" } else { "London, UK" })
            .bind(state).bind(0.95 - f64::from(u32::try_from(index % 10).expect("small index")) * 0.02)
            .execute(pool).await.expect("seed listing");
    }
    for offset in 0..6 {
        sqlx::query(
            "INSERT INTO events (listing_id, to_state, created_at) VALUES (?, 'submitted', ?)",
        )
        .bind(format!("workspace-{}", 12 + offset))
        .bind(chrono::Utc::now() - chrono::Duration::days(offset))
        .execute(pool)
        .await
        .expect("seed activity");
    }
}
