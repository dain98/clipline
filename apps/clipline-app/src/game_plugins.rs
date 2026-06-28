//! Game plugin registry.
//!
//! A game plugin describes how Clipline should recognize a game's real in-game
//! window from normal Win32 window/process metadata. Plugins must stay
//! anti-cheat-safe: no injection, no memory reads, no game-process hooks. The
//! installable package format is declarative; event ingestion stays behind
//! built-in capability names.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use clipline_capture::windows::CapturableWindow;
use clipline_events::GameId;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::markers::PollerMsg;
use crate::settings::{GamePluginSettings, GameRecordingMode, GameSettings};

pub const LEAGUE_OF_LEGENDS_ID: &str = "league_of_legends";
pub const LEAGUE_LIVE_CLIENT_EVENT_SOURCE: &str = "league_live_client";
pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const PLUGIN_MANIFEST_FILE: &str = "clipline-plugin.json";
pub const PLUGIN_RECEIPT_FILE: &str = "clipline-plugin.receipt.json";

pub type EventSourceSpawner = fn(GameEventSourceContext) -> Receiver<PollerMsg>;

#[derive(Clone, Debug)]
pub struct GameEventSourceContext {
    pub lol_url: Option<String>,
    pub recording_t0: Instant,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GamePluginInfo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub event_markers: bool,
    pub installed_version: Option<String>,
    pub seed_version: Option<String>,
    pub latest_version: Option<String>,
    pub latest_source_label: Option<String>,
    pub install_state: String,
    pub install_provenance: Option<String>,
    pub first_party: bool,
    pub update_available: bool,
    pub can_reset_to_seed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<serde_json::Value>,
    /// Icon for the UI: the plugin's bundled icon URL, or a cached icon
    /// extracted from the running game's executable. None when neither exists.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub package_version: Version,
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    #[serde(default)]
    pub icon: Option<PluginIcon>,
    pub window_match: WindowMatchRule,
    #[serde(default)]
    pub event_source: Option<String>,
    #[serde(default)]
    pub presentation: Option<serde_json::Value>,
}

impl PluginManifest {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != PLUGIN_MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "unsupported plugin manifest schema {}; expected {}",
                self.schema_version, PLUGIN_MANIFEST_SCHEMA_VERSION
            ));
        }
        validate_plugin_id(&self.id)?;
        if self.name.trim().is_empty() {
            return Err("plugin name is required".into());
        }
        if self.summary.trim().is_empty() {
            return Err("plugin summary is required".into());
        }
        self.window_match.validate()?;
        if let Some(event_source) = self.event_source.as_deref() {
            if event_source_spawner(event_source).is_none() {
                return Err(format!("unsupported plugin event source {event_source:?}"));
            }
        }
        Ok(())
    }

    pub fn match_window<'a>(
        &self,
        windows: &'a [CapturableWindow],
    ) -> Option<&'a CapturableWindow> {
        self.window_match.match_window(windows)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginIcon {
    UiAsset { path: String },
    File { path: String },
    DataUrl { data: String },
    Extracted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMatchRule {
    pub exe_name: String,
    #[serde(default = "default_window_selection")]
    pub selection: WindowSelection,
}

impl WindowMatchRule {
    fn validate(&self) -> Result<(), String> {
        if self.exe_name.trim().is_empty() {
            return Err("plugin window matcher exe_name is required".into());
        }
        Ok(())
    }

    fn match_window<'a>(&self, windows: &'a [CapturableWindow]) -> Option<&'a CapturableWindow> {
        let exe_name = self.exe_name.trim();
        match self.selection {
            WindowSelection::LongestTitle => windows
                .iter()
                .filter(|window| window.exe_name.eq_ignore_ascii_case(exe_name))
                .max_by_key(|window| window.title.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowSelection {
    LongestTitle,
}

fn default_window_selection() -> WindowSelection {
    WindowSelection::LongestTitle
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallProvenance {
    Seeded,
    Manual,
    Unknown,
}

impl InstallProvenance {
    fn as_str(self) -> &'static str {
        match self {
            Self::Seeded => "seeded",
            Self::Manual => "manual",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPluginRecord {
    pub plugin_id: String,
    pub package_version: Version,
    pub schema_version: u32,
    pub provenance: InstallProvenance,
    pub source_label: String,
    pub installed_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KnownFirstPartyPackageRelease {
    pub plugin_id: &'static str,
    pub version: Version,
    pub source_label: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

impl InstalledPluginRecord {
    pub fn new_for_seed(
        plugin_id: &str,
        package_version: Version,
        schema_version: u32,
        source_label: &str,
    ) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
            package_version,
            schema_version,
            provenance: InstallProvenance::Seeded,
            source_label: source_label.to_string(),
            installed_at_unix: unix_now(),
            package_hash: None,
            signature: None,
        }
    }

    #[cfg(test)]
    fn with_provenance(mut self, provenance: InstallProvenance) -> Self {
        self.provenance = provenance;
        self
    }
}

#[derive(Debug, Clone)]
pub struct GamePlugin {
    pub manifest: PluginManifest,
    pub install: InstalledPluginRecord,
    pub root_dir: Option<PathBuf>,
}

impl GamePlugin {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn default_settings(&self) -> GamePluginSettings {
        GamePluginSettings {
            enabled: self.manifest.default_enabled,
            recording_mode: self.manifest.default_recording_mode,
        }
    }

    pub fn settings(&self, settings: &GameSettings) -> GamePluginSettings {
        settings
            .plugins
            .get(self.id())
            .cloned()
            .unwrap_or_else(|| self.default_settings())
    }

    pub fn info(&self) -> GamePluginInfo {
        let seed_version = seed_package_version(self.id());
        let latest_release = known_first_party_package_release(self.id());
        let mut latest_package = seed_version
            .as_ref()
            .map(|version| (version.clone(), "bundled seed".to_string()));
        if let Some(release) = latest_release.as_ref() {
            let release_package = (release.version.clone(), release.source_label.to_string());
            if latest_package
                .as_ref()
                .is_none_or(|(version, _)| release_package.0 > *version)
            {
                latest_package = Some(release_package);
            }
        }
        let update_available = latest_package
            .as_ref()
            .is_some_and(|(version, _)| version > &self.manifest.package_version);
        let first_party = seed_version.is_some() || latest_release.is_some();
        GamePluginInfo {
            id: self.id().into(),
            name: self.manifest.name.clone(),
            summary: self.manifest.summary.clone(),
            default_enabled: self.manifest.default_enabled,
            default_recording_mode: self.manifest.default_recording_mode,
            event_markers: self
                .manifest
                .event_source
                .as_deref()
                .and_then(event_source_spawner)
                .is_some(),
            installed_version: Some(self.manifest.package_version.to_string()),
            seed_version: seed_version.as_ref().map(ToString::to_string),
            latest_version: latest_package
                .as_ref()
                .map(|(version, _)| version.to_string()),
            latest_source_label: latest_package.map(|(_, label)| label),
            install_state: install_state(&self.install).to_string(),
            install_provenance: Some(self.install.provenance.as_str().to_string()),
            first_party,
            update_available,
            can_reset_to_seed: first_party,
            presentation: self.presentation_value(),
            icon: self.icon_string(),
        }
    }

    pub fn match_window<'a>(
        &self,
        windows: &'a [CapturableWindow],
    ) -> Option<&'a CapturableWindow> {
        self.manifest.match_window(windows)
    }

    /// Resolve the plugin's icon for the UI: prefer the bundled icon, else a
    /// previously-cached icon extracted from the running game's executable.
    fn icon_string(&self) -> Option<String> {
        match self.manifest.icon.as_ref()? {
            PluginIcon::UiAsset { path } => Some(path.clone()),
            PluginIcon::DataUrl { data } => Some(data.clone()),
            PluginIcon::File { path } => self.package_file_data_url(path),
            PluginIcon::Extracted => {
                let cache = plugin_icon_cache_path(self.id())?;
                let bytes = std::fs::read(&cache).ok()?;
                Some(crate::game_icon::png_data_url(&bytes))
            }
        }
    }

    fn presentation_value(&self) -> Option<serde_json::Value> {
        let mut presentation = self.manifest.presentation.clone()?;
        let Some(marker_kinds) = presentation
            .get_mut("marker_kinds")
            .and_then(serde_json::Value::as_object_mut)
        else {
            return Some(presentation);
        };

        for config in marker_kinds.values_mut() {
            let Some(icon_value) = config.get_mut("icon") else {
                continue;
            };
            let Some(icon_path) = icon_value.as_str() else {
                continue;
            };
            let Some(data_url) = self.package_file_data_url(icon_path) else {
                continue;
            };
            *icon_value = serde_json::Value::String(data_url);
        }
        Some(presentation)
    }

    fn package_file_data_url(&self, path: &str) -> Option<String> {
        self.root_dir
            .as_deref()
            .and_then(|root| safe_relative_path(root, path))
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| crate::game_icon::png_data_url(&bytes))
    }

    fn uses_extracted_icon(&self) -> bool {
        matches!(self.manifest.icon, Some(PluginIcon::Extracted) | None)
    }
}

fn plugin_icon_cache_path(plugin_id: &str) -> Option<PathBuf> {
    // Plugin ids are simple slugs; reject anything that could escape the dir.
    if plugin_id.is_empty() || plugin_id.contains(['/', '\\', '.']) {
        return None;
    }
    Some(crate::settings::icon_cache_dir().join(format!("{plugin_id}.png")))
}

/// Cache an icon-less plugin's icon by extracting it from the running game's
/// executable. No-op for plugins that ship an icon or already have a cache.
/// Cheap to call on the detection poll loop: it short-circuits before any work.
pub fn ensure_plugin_icon_cached(plugin_id: &str, exe_path: &str) {
    let needs_extraction = all()
        .iter()
        .any(|plugin| plugin.id() == plugin_id && plugin.uses_extracted_icon());
    if !needs_extraction {
        return;
    }
    let Some(cache) = plugin_icon_cache_path(plugin_id) else {
        return;
    };
    if cache.exists() {
        return;
    }
    if let Some(png) = crate::game_icon::extract_exe_icon_png(exe_path) {
        if let Some(parent) = cache.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("create icon cache dir {parent:?}: {e}");
                return;
            }
        }
        if let Err(e) = std::fs::write(&cache, &png) {
            eprintln!("write icon cache {cache:?}: {e}");
        }
    }
}

pub fn all() -> Vec<GamePlugin> {
    let mut plugins = load_installed_plugins(&plugin_install_root()).unwrap_or_default();
    if !plugins
        .iter()
        .any(|plugin| plugin.id() == LEAGUE_OF_LEGENDS_ID)
    {
        let manifest = league_seed_manifest();
        let package_version = manifest.package_version.clone();
        plugins.push(GamePlugin {
            manifest,
            install: InstalledPluginRecord::new_for_seed(
                LEAGUE_OF_LEGENDS_ID,
                package_version,
                PLUGIN_MANIFEST_SCHEMA_VERSION,
                "bundled fallback",
            ),
            root_dir: None,
        });
    }
    plugins
}

pub fn contains(id: &str) -> bool {
    all().iter().any(|plugin| plugin.id() == id)
}

pub fn plugin_id_for_game_id(game_id: GameId) -> &'static str {
    match game_id {
        GameId::LeagueOfLegends => LEAGUE_OF_LEGENDS_ID,
        GameId::Valorant => "valorant",
        GameId::Cs2 => "cs2",
    }
}

pub fn display_name_for_game_id(game_id: GameId) -> &'static str {
    match game_id {
        GameId::LeagueOfLegends => "League of Legends",
        GameId::Valorant => "Valorant",
        GameId::Cs2 => "Cs2",
    }
}

pub fn seed_package_version(plugin_id: &str) -> Option<Version> {
    match plugin_id {
        LEAGUE_OF_LEGENDS_ID => Some(league_seed_manifest().package_version),
        _ => None,
    }
}

pub fn known_first_party_package_release(plugin_id: &str) -> Option<KnownFirstPartyPackageRelease> {
    match plugin_id {
        LEAGUE_OF_LEGENDS_ID => Some(KnownFirstPartyPackageRelease {
            plugin_id: LEAGUE_OF_LEGENDS_ID,
            version: Version::parse("1.3.0").expect("known League package version is valid"),
            source_label: "clipline-plugin-league-of-legends",
            url: "https://github.com/dain98/clipline-plugin-league-of-legends/releases/download/v1.3.0/clipline-plugin-league-of-legends-1.3.0.zip",
            sha256: "070687055eb04610820ba36c9506350c39984217e461b785747f16ab2ceb9390",
        }),
        _ => None,
    }
}

pub fn has_event_source(plugin_id: Option<&str>) -> bool {
    let Some(id) = plugin_id else {
        return false;
    };
    all().iter().any(|plugin| {
        plugin.id() == id
            && plugin
                .manifest
                .event_source
                .as_deref()
                .and_then(event_source_spawner)
                .is_some()
    })
}

pub fn spawn_event_source(
    plugin_id: Option<&str>,
    context: GameEventSourceContext,
) -> Option<Receiver<PollerMsg>> {
    let id = plugin_id?;
    let plugin = all().into_iter().find(|plugin| plugin.id() == id)?;
    let spawn = plugin
        .manifest
        .event_source
        .as_deref()
        .and_then(event_source_spawner)?;
    Some(spawn(context))
}

pub fn plugin_install_root() -> PathBuf {
    crate::settings::persistence::config_base().join("plugins")
}

pub fn seed_bundled_plugins(seed_root: &Path) -> Result<Vec<SeedOutcome>, String> {
    let install_root = plugin_install_root();
    seed_first_party_plugins(seed_root, &install_root)
}

pub fn seed_first_party_plugins(
    seed_root: &Path,
    install_root: &Path,
) -> Result<Vec<SeedOutcome>, String> {
    let league_seed = seed_root.join(LEAGUE_OF_LEGENDS_ID);
    if !league_seed.exists() {
        return Ok(Vec::new());
    }
    seed_plugin_from_dir(&league_seed, install_root, "bundled").map(|outcome| vec![outcome])
}

pub fn reset_first_party_plugin_to_seed(
    plugin_id: &str,
    seed_root: &Path,
    install_root: &Path,
) -> Result<SeedOutcome, String> {
    if plugin_id != LEAGUE_OF_LEGENDS_ID {
        return Err(format!("unknown first-party game plugin {plugin_id:?}"));
    }
    let seed_dir = seed_root.join(plugin_id);
    if !seed_dir.exists() {
        return Err(format!("bundled seed package is missing for {plugin_id}"));
    }
    let manifest = load_manifest(&seed_dir)?;
    let target = install_root.join(&manifest.id);
    let existed = target.exists();
    replace_plugin_dir(&seed_dir, &target)?;
    write_seed_receipt(&target, &manifest, "bundled reset")?;
    Ok(if existed {
        SeedOutcome::UpdatedSeeded
    } else {
        SeedOutcome::Installed
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    Installed,
    UpdatedSeeded,
    SkippedCurrent,
    SkippedManual,
    SkippedUnknown,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualInstallOutcome {
    Installed,
    Updated,
}

pub fn seed_plugin_from_dir(
    seed_dir: &Path,
    install_root: &Path,
    source_label: &str,
) -> Result<SeedOutcome, String> {
    let seed_manifest = load_manifest(seed_dir)?;
    let target = install_root.join(&seed_manifest.id);
    if target.exists() {
        let installed = load_installed_plugin(&target)?;
        match installed.install.provenance {
            InstallProvenance::Manual => return Ok(SeedOutcome::SkippedManual),
            InstallProvenance::Unknown => return Ok(SeedOutcome::SkippedUnknown),
            InstallProvenance::Seeded => {
                if installed.manifest.package_version >= seed_manifest.package_version {
                    return Ok(SeedOutcome::SkippedCurrent);
                }
            }
        }
        replace_plugin_dir(seed_dir, &target)?;
        write_seed_receipt(&target, &seed_manifest, source_label)?;
        return Ok(SeedOutcome::UpdatedSeeded);
    }

    replace_plugin_dir(seed_dir, &target)?;
    write_seed_receipt(&target, &seed_manifest, source_label)?;
    Ok(SeedOutcome::Installed)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("open plugin package for hashing {path:?}: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buf)
            .map_err(|e| format!("read plugin package for hashing {path:?}: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn install_verified_package_zip(
    zip_path: &Path,
    expected_sha256: &str,
    install_root: &Path,
    source_label: &str,
) -> Result<ManualInstallOutcome, String> {
    let actual_sha256 = sha256_file(zip_path)?;
    if actual_sha256 != expected_sha256.trim().to_ascii_lowercase() {
        return Err(format!(
            "plugin package digest mismatch: expected {}, got {}",
            expected_sha256, actual_sha256
        ));
    }

    std::fs::create_dir_all(install_root)
        .map_err(|e| format!("create plugin install root {install_root:?}: {e}"))?;
    let staging = install_root.join(format!(
        ".staging-plugin-{}-{}",
        std::process::id(),
        unix_now()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("remove stale plugin staging dir {staging:?}: {e}"))?;
    }

    let result = (|| {
        extract_plugin_zip(zip_path, &staging)?;
        let manifest = load_manifest(&staging)?;
        if manifest.id != LEAGUE_OF_LEGENDS_ID {
            return Err(format!(
                "unsupported first-party plugin package {:?}",
                manifest.id
            ));
        }
        write_manual_receipt(&staging, &manifest, source_label, &actual_sha256)?;
        activate_staged_plugin(&staging, install_root, &manifest)
    })();

    if result.is_err() && staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

pub fn load_installed_plugins(root: &Path) -> Result<Vec<GamePlugin>, String> {
    let mut plugins = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(plugins);
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_installed_plugin(&path) {
            Ok(plugin) => plugins.push(plugin),
            Err(e) => eprintln!("skip plugin package {path:?}: {e}"),
        }
    }
    plugins.sort_by(|a, b| a.id().cmp(b.id()));
    Ok(plugins)
}

pub fn load_installed_plugin(dir: &Path) -> Result<GamePlugin, String> {
    let manifest = load_manifest(dir)?;
    let install = load_receipt(dir, &manifest).unwrap_or_else(|| InstalledPluginRecord {
        plugin_id: manifest.id.clone(),
        package_version: manifest.package_version.clone(),
        schema_version: manifest.schema_version,
        provenance: InstallProvenance::Unknown,
        source_label: "unknown".into(),
        installed_at_unix: 0,
        package_hash: None,
        signature: None,
    });
    Ok(GamePlugin {
        manifest,
        install,
        root_dir: Some(dir.to_path_buf()),
    })
}

pub fn league_seed_manifest() -> PluginManifest {
    PluginManifest::from_json(LEAGUE_SEED_MANIFEST_JSON).expect("bundled League manifest is valid")
}

fn load_manifest(dir: &Path) -> Result<PluginManifest, String> {
    let path = dir.join(PLUGIN_MANIFEST_FILE);
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("read plugin manifest {path:?}: {e}"))?;
    PluginManifest::from_json(&json)
}

fn load_receipt(dir: &Path, manifest: &PluginManifest) -> Option<InstalledPluginRecord> {
    let path = dir.join(PLUGIN_RECEIPT_FILE);
    let json = std::fs::read_to_string(path).ok()?;
    let receipt: InstalledPluginRecord = serde_json::from_str(&json).ok()?;
    if receipt.plugin_id != manifest.id
        || receipt.schema_version != manifest.schema_version
        || receipt.package_version != manifest.package_version
    {
        return None;
    }
    Some(receipt)
}

fn write_seed_receipt(
    dir: &Path,
    manifest: &PluginManifest,
    source_label: &str,
) -> Result<(), String> {
    let receipt = InstalledPluginRecord::new_for_seed(
        &manifest.id,
        manifest.package_version.clone(),
        manifest.schema_version,
        source_label,
    );
    let json = serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(PLUGIN_RECEIPT_FILE), json)
        .map_err(|e| format!("write plugin receipt: {e}"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn extract_plugin_zip(zip_path: &Path, staging: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("open plugin package zip {zip_path:?}: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("read plugin package zip: {e}"))?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|e| format!("read plugin zip entry {index}: {e}"))?;
        let Some(path) = file.enclosed_name() else {
            return Err(format!("unsafe path in plugin zip entry {:?}", file.name()));
        };
        entries.push((index, path, file.is_dir()));
    }

    std::fs::create_dir_all(staging)
        .map_err(|e| format!("create plugin staging dir {staging:?}: {e}"))?;
    for (index, relative_path, is_dir) in entries {
        let out_path = staging.join(relative_path);
        if is_dir {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("create plugin zip directory {out_path:?}: {e}"))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create plugin zip parent {parent:?}: {e}"))?;
        }
        let mut file = archive
            .by_index(index)
            .map_err(|e| format!("read plugin zip entry {index}: {e}"))?;
        let mut out = std::fs::File::create(&out_path)
            .map_err(|e| format!("create plugin zip file {out_path:?}: {e}"))?;
        std::io::copy(&mut file, &mut out)
            .map_err(|e| format!("extract plugin zip file {out_path:?}: {e}"))?;
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn write_manual_receipt(
    dir: &Path,
    manifest: &PluginManifest,
    source_label: &str,
    package_hash: &str,
) -> Result<(), String> {
    let receipt = InstalledPluginRecord {
        plugin_id: manifest.id.clone(),
        package_version: manifest.package_version.clone(),
        schema_version: manifest.schema_version,
        provenance: InstallProvenance::Manual,
        source_label: source_label.to_string(),
        installed_at_unix: unix_now(),
        package_hash: Some(format!("sha256:{package_hash}")),
        signature: None,
    };
    let json = serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(PLUGIN_RECEIPT_FILE), json)
        .map_err(|e| format!("write plugin receipt: {e}"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn activate_staged_plugin(
    staging: &Path,
    install_root: &Path,
    manifest: &PluginManifest,
) -> Result<ManualInstallOutcome, String> {
    let target = install_root.join(&manifest.id);
    let outcome = if target.exists() {
        ManualInstallOutcome::Updated
    } else {
        ManualInstallOutcome::Installed
    };
    if !target.exists() {
        std::fs::rename(staging, &target)
            .map_err(|e| format!("activate plugin package {target:?}: {e}"))?;
        return Ok(outcome);
    }

    let backup = install_root.join(format!(".backup-plugin-{}-{}", manifest.id, unix_now()));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|e| format!("remove stale plugin backup {backup:?}: {e}"))?;
    }
    std::fs::rename(&target, &backup)
        .map_err(|e| format!("stage old plugin package backup {target:?}: {e}"))?;
    match std::fs::rename(staging, &target) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_dir_all(&backup) {
                eprintln!("remove old plugin backup {backup:?}: {e}");
            }
            Ok(outcome)
        }
        Err(e) => {
            let restore = std::fs::rename(&backup, &target);
            match restore {
                Ok(()) => Err(format!("activate plugin package {target:?}: {e}")),
                Err(restore_err) => Err(format!(
                    "activate plugin package {target:?}: {e}; restore backup failed: {restore_err}"
                )),
            }
        }
    }
}

fn replace_plugin_dir(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|e| format!("remove old plugin package: {e}"))?;
    }
    copy_dir_all(source, target)
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("create plugin package dir: {e}"))?;
    for entry in std::fs::read_dir(source).map_err(|e| format!("read plugin seed dir: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            std::fs::copy(&source_path, &target_path)
                .map_err(|e| format!("copy plugin package file: {e}"))?;
        }
    }
    Ok(())
}

fn safe_relative_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() || relative.contains("..") {
        return None;
    }
    Some(root.join(path))
}

fn event_source_spawner(name: &str) -> Option<EventSourceSpawner> {
    match name {
        LEAGUE_LIVE_CLIENT_EVENT_SOURCE => Some(league_of_legends::spawn_event_source),
        _ => None,
    }
}

fn validate_plugin_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.contains(['/', '\\', '.'])
        || id
            .chars()
            .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
    {
        return Err(format!("invalid plugin id {id:?}"));
    }
    Ok(())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn install_state(receipt: &InstalledPluginRecord) -> &'static str {
    match receipt.provenance {
        InstallProvenance::Seeded | InstallProvenance::Manual => "installed",
        InstallProvenance::Unknown => "repair_available",
    }
}

const LEAGUE_SEED_MANIFEST_JSON: &str = r#"{
  "schema_version": 1,
  "package_version": "1.2.1",
  "id": "league_of_legends",
  "name": "League of Legends",
  "summary": "Auto-records full matches when the in-game window is active.",
  "default_enabled": true,
  "default_recording_mode": "full_session",
  "icon": { "type": "file", "path": "assets/games/league-of-legends.png" },
  "window_match": { "exe_name": "League of Legends.exe", "selection": "longest_title" },
  "event_source": "league_live_client",
  "presentation": {
    "marker_kinds": {
      "ChampionKill": { "category": "kill", "icon": "assets/markers/kill.png" },
      "ChampionDeath": { "category": "death", "icon": "assets/markers/death.png" },
      "DragonKill": { "category": "objective", "icon": "assets/markers/dragon.png" },
      "BaronKill": { "category": "objective", "icon": "assets/markers/baron.png" },
      "TurretKilled": { "category": "structure", "icon": "assets/markers/turret.png" }
    },
    "marker_categories": {
      "kill": { "singular": "kill", "plural": "kills", "glyph": "✕" },
      "death": { "singular": "death", "plural": "deaths", "glyph": "✕" },
      "spree": { "singular": "spree", "plural": "sprees", "glyph": "★" },
      "objective": { "singular": "objective", "plural": "objectives", "glyph": "◆" },
      "structure": { "singular": "structure", "plural": "structures", "glyph": "▣" },
      "info": { "singular": "event", "plural": "events", "glyph": "•" }
    },
    "gallery": {
      "summary": "player_summary_kda",
      "full_session_title": "summary"
    },
    "event_rail": {
      "enabled": true,
      "title": "Match events"
    },
    "metadata_panel": {
      "enabled": true,
      "fields": [
        { "type": "portrait", "source": "player_summary.champion_name", "label": "Champion" },
        { "type": "champion", "source": "player_summary.champion_name", "label": "Champion" },
        { "type": "kda", "label": "K/D/A" }
      ]
    }
  }
}"#;

mod league_of_legends {
    use std::sync::mpsc::Receiver;

    use super::GameEventSourceContext;
    use crate::markers::PollerMsg;

    pub fn spawn_event_source(context: GameEventSourceContext) -> Receiver<PollerMsg> {
        crate::markers::spawn(context.lol_url, context.recording_t0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_capture::windows::CapturableWindow;
    use clipline_test_utils::TestDir;

    fn window(
        handle: isize,
        title: &str,
        exe_name: &str,
        exe_path: Option<&str>,
    ) -> CapturableWindow {
        CapturableWindow {
            handle,
            title: title.into(),
            process_id: handle as u32,
            exe_name: exe_name.into(),
            exe_path: exe_path.map(str::to_string),
        }
    }

    #[test]
    fn manifest_rejects_unsupported_schema_versions() {
        let json = r#"{
          "schema_version": 2,
          "package_version": "1.0.0",
          "id": "league_of_legends",
          "name": "League of Legends",
          "summary": "Auto-records full matches when the in-game window is active.",
          "default_enabled": true,
          "default_recording_mode": "full_session",
          "window_match": { "exe_name": "League of Legends.exe", "selection": "longest_title" },
          "event_source": "league_live_client"
        }"#;

        let err = PluginManifest::from_json(json).unwrap_err();

        assert!(err.contains("unsupported plugin manifest schema"), "{err}");
    }

    #[test]
    fn declarative_league_matcher_preserves_longest_title_behavior() {
        let manifest = league_seed_manifest();
        let windows = vec![
            window(1, "League of Legends", "LeagueClientUx.exe", None),
            window(2, "League", "League of Legends.exe", None),
            window(
                3,
                "League of Legends (TM) Client",
                "League of Legends.exe",
                None,
            ),
        ];

        let matched = manifest.match_window(&windows).expect("game window");

        assert_eq!(matched.handle, 3);
        assert_eq!(matched.exe_name, "League of Legends.exe");
    }

    #[test]
    fn seed_does_not_clobber_manual_or_unknown_installs() {
        let dir = TestDir::new("clipline-plugin", "seed-provenance");
        let seed = dir.path().join("seed");
        let install = dir.path().join("install");
        write_manifest(&seed, "1.2.0");
        write_manifest(&install.join(LEAGUE_OF_LEGENDS_ID), "1.0.0");
        write_receipt(
            &install.join(LEAGUE_OF_LEGENDS_ID),
            InstallProvenance::Manual,
            "1.0.0",
        );

        let manual = seed_plugin_from_dir(&seed, &install, "bundled").unwrap();
        assert_eq!(manual, SeedOutcome::SkippedManual);

        std::fs::write(
            install.join(LEAGUE_OF_LEGENDS_ID).join(PLUGIN_RECEIPT_FILE),
            b"not json",
        )
        .unwrap();
        let unknown = seed_plugin_from_dir(&seed, &install, "bundled").unwrap();
        assert_eq!(unknown, SeedOutcome::SkippedUnknown);
    }

    #[test]
    fn seed_updates_only_older_seeded_installs() {
        let dir = TestDir::new("clipline-plugin", "seed-version");
        let seed = dir.path().join("seed");
        let install = dir.path().join("install");
        write_manifest(&seed, "1.2.0");
        write_manifest(&install.join(LEAGUE_OF_LEGENDS_ID), "1.0.0");
        write_receipt(
            &install.join(LEAGUE_OF_LEGENDS_ID),
            InstallProvenance::Seeded,
            "1.0.0",
        );

        let outcome = seed_plugin_from_dir(&seed, &install, "bundled").unwrap();

        assert_eq!(outcome, SeedOutcome::UpdatedSeeded);
        let loaded = load_installed_plugin(&install.join(LEAGUE_OF_LEGENDS_ID)).unwrap();
        assert_eq!(loaded.manifest.package_version.to_string(), "1.2.0");
        assert_eq!(loaded.install.provenance, InstallProvenance::Seeded);
    }

    #[test]
    fn game_id_bridge_keeps_league_plugin_id_stable() {
        assert_eq!(
            plugin_id_for_game_id(clipline_events::GameId::LeagueOfLegends),
            LEAGUE_OF_LEGENDS_ID
        );
        assert_eq!(
            display_name_for_game_id(clipline_events::GameId::LeagueOfLegends),
            "League of Legends"
        );
    }

    #[test]
    fn league_presentation_styles_exactly_timeline_marker_kinds() {
        let manifest = league_seed_manifest();
        let presentation = manifest.presentation.as_ref().expect("presentation block");
        let marker_kinds = presentation
            .get("marker_kinds")
            .and_then(serde_json::Value::as_object)
            .expect("marker kind map");
        let styled = marker_kinds
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        let kept = [
            clipline_events::EventKind::ChampionKill,
            clipline_events::EventKind::ChampionDeath,
            clipline_events::EventKind::TurretKilled,
            clipline_events::EventKind::DragonKill,
            clipline_events::EventKind::BaronKill,
        ]
        .into_iter()
        .filter(|kind| timeline_policy_keeps(*kind))
        .map(event_kind_name)
        .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(styled, kept);
    }

    #[test]
    fn plugin_info_resolves_packaged_presentation_assets() {
        let dir = TestDir::new("clipline-plugin", "presentation-assets");
        let plugin_dir = dir.path().join(LEAGUE_OF_LEGENDS_ID);
        std::fs::create_dir_all(plugin_dir.join("assets/games")).unwrap();
        std::fs::create_dir_all(plugin_dir.join("assets/markers")).unwrap();
        std::fs::write(
            plugin_dir.join("assets/games/league-of-legends.png"),
            b"game-icon",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("assets/markers/kill.png"), b"kill-icon").unwrap();
        std::fs::write(
            plugin_dir.join(PLUGIN_MANIFEST_FILE),
            r#"{
              "schema_version": 1,
              "package_version": "1.2.0",
              "id": "league_of_legends",
              "name": "League of Legends",
              "summary": "Auto-records full matches when the in-game window is active.",
              "default_enabled": true,
              "default_recording_mode": "full_session",
              "icon": { "type": "file", "path": "assets/games/league-of-legends.png" },
              "window_match": { "exe_name": "League of Legends.exe", "selection": "longest_title" },
              "event_source": "league_live_client",
              "presentation": {
                "marker_kinds": {
                  "ChampionKill": { "category": "kill", "icon": "assets/markers/kill.png" }
                }
              }
            }"#,
        )
        .unwrap();
        write_receipt(&plugin_dir, InstallProvenance::Manual, "1.2.0");

        let info = load_installed_plugin(&plugin_dir).unwrap().info();
        let presentation = info.presentation.expect("presentation");
        let marker_icon = presentation
            .get("marker_kinds")
            .and_then(|value| value.get("ChampionKill"))
            .and_then(|value| value.get("icon"))
            .and_then(serde_json::Value::as_str)
            .expect("marker icon");

        assert!(
            info.icon
                .as_deref()
                .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
            "plugin package icon should be exposed as a data URL"
        );
        assert!(
            marker_icon.starts_with("data:image/png;base64,"),
            "presentation marker icons should be package-relative data URLs, got {marker_icon}"
        );
    }

    #[test]
    fn first_party_release_status_uses_external_package_feed() {
        let release = known_first_party_package_release(LEAGUE_OF_LEGENDS_ID)
            .expect("League has a first-party package release");
        let manifest = league_seed_manifest();
        let plugin = GamePlugin {
            manifest: manifest.clone(),
            install: InstalledPluginRecord::new_for_seed(
                &manifest.id,
                manifest.package_version,
                manifest.schema_version,
                "bundled",
            ),
            root_dir: None,
        };

        let info = plugin.info();

        assert_eq!(release.version.to_string(), "1.3.0");
        assert_eq!(info.latest_version.as_deref(), Some("1.3.0"));
        assert_eq!(
            info.latest_source_label.as_deref(),
            Some("clipline-plugin-league-of-legends")
        );
        assert!(
            info.update_available,
            "external package release should make the older bundled seed updatable"
        );
    }

    #[test]
    fn reset_to_seed_explicitly_clobbers_unknown_installs() {
        let dir = TestDir::new("clipline-plugin", "reset-to-seed");
        let seed_root = dir.path().join("seed-root");
        let seed = seed_root.join(LEAGUE_OF_LEGENDS_ID);
        let install = dir.path().join("install");
        let target = install.join(LEAGUE_OF_LEGENDS_ID);
        write_manifest(&seed, "1.2.0");
        write_manifest(&target, "9.9.9");
        std::fs::write(target.join(PLUGIN_RECEIPT_FILE), b"not json").unwrap();

        let outcome =
            reset_first_party_plugin_to_seed(LEAGUE_OF_LEGENDS_ID, &seed_root, &install).unwrap();

        assert_eq!(outcome, SeedOutcome::UpdatedSeeded);
        let loaded = load_installed_plugin(&target).unwrap();
        assert_eq!(loaded.manifest.package_version.to_string(), "1.2.0");
        assert_eq!(loaded.install.provenance, InstallProvenance::Seeded);
    }

    #[test]
    fn manual_zip_install_rejects_bad_digest_without_touching_active_package() {
        let dir = TestDir::new("clipline-plugin", "zip-bad-digest");
        let zip = dir.path().join("league.zip");
        let install = dir.path().join("install");
        write_manifest(&install.join(LEAGUE_OF_LEGENDS_ID), "1.0.0");
        write_receipt(
            &install.join(LEAGUE_OF_LEGENDS_ID),
            InstallProvenance::Seeded,
            "1.0.0",
        );
        write_plugin_zip(&zip, "1.2.0", LEAGUE_LIVE_CLIENT_EVENT_SOURCE, &[]);

        let err =
            install_verified_package_zip(&zip, "not-the-real-digest", &install, "test release")
                .unwrap_err();

        assert!(err.contains("plugin package digest"), "{err}");
        let loaded = load_installed_plugin(&install.join(LEAGUE_OF_LEGENDS_ID)).unwrap();
        assert_eq!(loaded.manifest.package_version.to_string(), "1.0.0");
    }

    #[test]
    fn manual_zip_install_rejects_corrupt_zip_without_touching_active_package() {
        let dir = TestDir::new("clipline-plugin", "zip-corrupt");
        let zip = dir.path().join("league.zip");
        let install = dir.path().join("install");
        write_manifest(&install.join(LEAGUE_OF_LEGENDS_ID), "1.0.0");
        write_receipt(
            &install.join(LEAGUE_OF_LEGENDS_ID),
            InstallProvenance::Seeded,
            "1.0.0",
        );
        std::fs::write(&zip, b"not a zip").unwrap();
        let digest = sha256_file(&zip).unwrap();

        let err =
            install_verified_package_zip(&zip, &digest, &install, "test release").unwrap_err();

        assert!(err.contains("read plugin package zip"), "{err}");
        let loaded = load_installed_plugin(&install.join(LEAGUE_OF_LEGENDS_ID)).unwrap();
        assert_eq!(loaded.manifest.package_version.to_string(), "1.0.0");
    }

    #[test]
    fn manual_zip_install_rejects_zip_slip_entries() {
        let dir = TestDir::new("clipline-plugin", "zip-slip");
        let zip = dir.path().join("league.zip");
        let install = dir.path().join("install");
        write_plugin_zip(
            &zip,
            "1.2.0",
            LEAGUE_LIVE_CLIENT_EVENT_SOURCE,
            &[("../evil.txt", "x")],
        );
        let digest = sha256_file(&zip).unwrap();

        let err =
            install_verified_package_zip(&zip, &digest, &install, "test release").unwrap_err();

        assert!(err.contains("unsafe path"), "{err}");
        assert!(!install.join(LEAGUE_OF_LEGENDS_ID).exists());
    }

    #[test]
    fn manual_zip_install_rejects_unknown_event_source_capabilities() {
        let dir = TestDir::new("clipline-plugin", "zip-unknown-capability");
        let zip = dir.path().join("league.zip");
        let install = dir.path().join("install");
        write_plugin_zip(&zip, "1.2.0", "future_event_source", &[]);
        let digest = sha256_file(&zip).unwrap();

        let err =
            install_verified_package_zip(&zip, &digest, &install, "test release").unwrap_err();

        assert!(err.contains("unsupported plugin event source"), "{err}");
        assert!(!install.join(LEAGUE_OF_LEGENDS_ID).exists());
    }

    fn write_manifest(dir: &std::path::Path, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(PLUGIN_MANIFEST_FILE),
            format!(
                r#"{{
                  "schema_version": 1,
                  "package_version": "{version}",
                  "id": "league_of_legends",
                  "name": "League of Legends",
                  "summary": "Auto-records full matches when the in-game window is active.",
                  "default_enabled": true,
                  "default_recording_mode": "full_session",
                  "icon": {{ "type": "ui_asset", "path": "assets/games/league-of-legends.png" }},
                  "window_match": {{ "exe_name": "League of Legends.exe", "selection": "longest_title" }},
                  "event_source": "league_live_client"
                }}"#
            ),
        )
        .unwrap();
    }

    fn write_receipt(dir: &std::path::Path, provenance: InstallProvenance, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let receipt = InstalledPluginRecord::new_for_seed(
            LEAGUE_OF_LEGENDS_ID,
            semver::Version::parse(version).unwrap(),
            1,
            "test",
        )
        .with_provenance(provenance);
        std::fs::write(
            dir.join(PLUGIN_RECEIPT_FILE),
            serde_json::to_string_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }

    fn event_kind_name(kind: clipline_events::EventKind) -> String {
        serde_json::to_value(kind)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    fn timeline_policy_keeps(kind: clipline_events::EventKind) -> bool {
        clipline_events::is_timeline_marker(&clipline_events::GameEvent {
            game_id: clipline_events::GameId::LeagueOfLegends,
            kind,
            actor: "Player".into(),
            victim: Some("Enemy".into()),
            assisters: Vec::new(),
            subtype: None,
            game_time_s: 60.0,
            recording_offset_s: Some(12.0),
            importance: 5,
            involves_local_player: true,
        })
    }

    fn write_plugin_zip(
        path: &std::path::Path,
        version: &str,
        event_source: &str,
        extra_entries: &[(&str, &str)],
    ) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(PLUGIN_MANIFEST_FILE, options).unwrap();
        use std::io::Write;
        write!(
            zip,
            r#"{{
              "schema_version": 1,
              "package_version": "{version}",
              "id": "league_of_legends",
              "name": "League of Legends",
              "summary": "Auto-records full matches when the in-game window is active.",
              "default_enabled": true,
              "default_recording_mode": "full_session",
              "icon": {{ "type": "ui_asset", "path": "assets/games/league-of-legends.png" }},
              "window_match": {{ "exe_name": "League of Legends.exe", "selection": "longest_title" }},
              "event_source": "{event_source}"
            }}"#
        )
        .unwrap();
        for (name, body) in extra_entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
}
