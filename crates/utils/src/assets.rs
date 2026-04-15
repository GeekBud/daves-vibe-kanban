use rust_embed::RustEmbed;

pub fn asset_dir() -> std::path::PathBuf {
    // 1. Allow override via VK_ASSET_DIR (legacy env var)
    if let Ok(dir) = std::env::var("VK_ASSET_DIR") {
        let path = std::path::PathBuf::from(dir);
        if !path.exists() {
            std::fs::create_dir_all(&path).expect("Failed to create VK_ASSET_DIR");
        }
        return path;
    }

    // 2. Read from daves_env_config.json
    let cfg = crate::env_config::load_config();
    let path = if let Some(dir) = &cfg.paths.asset_dir {
        std::path::PathBuf::from(dir)
    } else {
        std::path::PathBuf::from("/Users/lianghusile/dave/appData/daves-vibe-kanban")
    };

    // Ensure the directory exists
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("Failed to create asset directory");
    }

    path
}

pub fn prod_asset_dir_path() -> std::path::PathBuf {
    std::path::PathBuf::from("/Users/lianghusile/dave/appData/daves-vibe-kanban")
}

pub fn config_path() -> std::path::PathBuf {
    asset_dir().join("config.json")
}

pub fn profiles_path() -> std::path::PathBuf {
    asset_dir().join("profiles.json")
}

pub fn credentials_path() -> std::path::PathBuf {
    asset_dir().join("credentials.json")
}

pub fn trusted_keys_path() -> std::path::PathBuf {
    asset_dir().join("trusted_ed25519_public_keys.json")
}

pub fn server_signing_key_path() -> std::path::PathBuf {
    asset_dir().join("server_ed25519_signing_key")
}

pub fn relay_host_credentials_path() -> std::path::PathBuf {
    asset_dir().join("relay_host_credentials.json")
}

#[derive(RustEmbed)]
#[folder = "../../assets/sounds"]
pub struct SoundAssets;

#[derive(RustEmbed)]
#[folder = "../../assets/scripts"]
pub struct ScriptAssets;
