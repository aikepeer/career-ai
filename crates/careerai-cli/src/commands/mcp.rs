//! `careerai mcp probe` — read-only probe of configured MCP-source
//! servers. Reports per-source: tool count, whether a known job-search
//! tool name is advertised, and any spawn / handshake error. Never
//! sends a real query.

use anyhow::Result;

use careerai_core::config::CoreConfig;

pub async fn run_probe(cfg: &CoreConfig) -> Result<()> {
    if cfg.sources.mcp.is_empty() {
        println!("no `sources.mcp` entries configured");
        return Ok(());
    }
    let mut any_unreachable = false;
    for src_cfg in &cfg.sources.mcp {
        if !src_cfg.enabled {
            println!(
                "{name}: (disabled) command={cmd} args={args:?}",
                name = src_cfg.name,
                cmd = src_cfg.mcp.command,
                args = src_cfg.mcp.args,
            );
            continue;
        }
        match careerai_sources::probe_mcp_source(src_cfg).await {
            Ok(report) => {
                let matched = report.matched_tool.as_deref().map_or_else(
                    || "no job-search tool".to_string(),
                    |t| format!("{t} available"),
                );
                println!(
                    "{name}: reachable ({count} tools, {matched})",
                    name = report.source_name,
                    count = report.tool_count,
                );
                if report.tool_count <= 12 {
                    println!("  tools: {:?}", report.tools);
                }
            }
            Err(e) => {
                any_unreachable = true;
                println!("{name}: unreachable -- {err}", name = src_cfg.name, err = e);
                println!(
                    "  hint: install/run `{cmd} {args}` and retry",
                    cmd = src_cfg.mcp.command,
                    args = src_cfg.mcp.args.join(" "),
                );
            }
        }
    }
    if any_unreachable {
        std::process::exit(2);
    }
    Ok(())
}
