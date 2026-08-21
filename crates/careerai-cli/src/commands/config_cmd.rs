//! Subcommand handler for `careerai config generate`.

use anyhow::{Context, Result};
use careerai_profile::schema::Profile;

pub fn run_generate(force: bool) -> Result<()> {
    let cwd = careerai_core::paths::resolve_root_env();
    let profile_path = cwd.join("profile").join("profile.yaml");
    let local_cfg_path = cwd.join("config").join("local.yaml");

    if local_cfg_path.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            local_cfg_path.display()
        );
    }

    let profile = if profile_path.exists() {
        let content = std::fs::read_to_string(&profile_path)
            .with_context(|| format!("read {}", profile_path.display()))?;
        serde_yaml::from_str::<Profile>(&content)
            .with_context(|| format!("parse {}", profile_path.display()))?
    } else {
        Profile::default()
    };

    let generated_yaml = careerai_profile::generate_config_yaml(&profile);

    if let Some(parent) = local_cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&local_cfg_path, &generated_yaml)
        .with_context(|| format!("write {}", local_cfg_path.display()))?;

    println!(
        "✅ Generated exhaustive configuration at {}",
        local_cfg_path.display()
    );
    Ok(())
}
