use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tracing::warn;
use urlencoding::encode;

use starbyte_core::{
    cartridge::{Cartridge, Region},
    manifest::{AssetConfig, LibraryViewMode, ProviderSettings, RuntimeConfig},
};

/// Stable library identifier derived from a normalized game title.
pub type GameId = String;

/// Local ROM information discovered during library scans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRomInfo {
    /// Stable game identifier derived from normalized title.
    pub game_id: GameId,
    /// Human-facing title from the ROM header.
    pub title: String,
    /// Normalized title used for merges and lookups.
    pub normalized_title: String,
    /// On-disk ROM path.
    pub rom_path: PathBuf,
    /// Mapper name reported by the cartridge header.
    pub mapper: String,
    /// Coprocessor family if detected.
    pub coprocessor: Option<String>,
    /// Cartridge region.
    pub region: String,
    /// File size in bytes.
    pub file_size_bytes: u64,
}

/// Remote metadata record cached for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMetadata {
    /// Stable game identifier derived from normalized title.
    pub game_id: GameId,
    /// Human-facing title from the provider.
    pub title: String,
    /// Normalized title used for merges and lookups.
    pub normalized_title: String,
    /// Provider/source label.
    pub source: String,
    /// Remote cover-art URL if one is known.
    pub cover_url: Option<String>,
    /// Whether the provider exposes cheat files for this title.
    pub has_cheat_files: bool,
    /// Provider cache timestamp.
    pub fetched_at_unix: u64,
}

/// Cached cover-art description for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverAsset {
    /// Stable game identifier derived from normalized title.
    pub game_id: GameId,
    /// Local cached image path.
    pub cache_path: PathBuf,
    /// Remote cover-art source URL.
    pub source_url: String,
    /// Download timestamp.
    pub fetched_at_unix: u64,
}

/// One cheat entry associated with a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheatEntry {
    /// Stable cheat identifier.
    pub id: String,
    /// Stable game identifier derived from normalized title.
    pub game_id: GameId,
    /// Human-facing cheat name.
    pub name: String,
    /// Cheat code payload.
    pub code: String,
    /// Provider/source label.
    pub source: String,
    /// Cheat type or family label.
    pub kind: String,
    /// Whether this cheat is enabled in local user settings.
    pub enabled: bool,
}

/// Installed/missing state for one library entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledStatus {
    /// A local ROM exists for the entry.
    Installed,
    /// The entry only exists via cached metadata.
    Missing,
}

/// Fully merged library entry exposed to CLI and GUI shells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    /// Stable game identifier derived from normalized title.
    pub game_id: GameId,
    /// Best display title available for the game.
    pub display_title: String,
    /// Installed/missing state.
    pub installed_status: InstalledStatus,
    /// Local ROM information if present.
    pub local: Option<LocalRomInfo>,
    /// Cached remote metadata if present.
    pub metadata: Option<GameMetadata>,
    /// Cached cover-art description if present.
    pub cover: Option<CoverAsset>,
    /// Cached cheats if present.
    pub cheats: Vec<CheatEntry>,
}

/// Library filtering controls shared by CLI and GUI surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFilter {
    /// Free-text title query.
    pub query: String,
    /// Whether only installed entries should be shown.
    pub installed_only: bool,
    /// Active presentation mode.
    pub view_mode: LibraryViewMode,
}

impl Default for LibraryFilter {
    fn default() -> Self {
        Self {
            query: String::new(),
            installed_only: false,
            view_mode: LibraryViewMode::Grid,
        }
    }
}

/// Read-only library snapshot exported to CLI and GUI shells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    /// Entries matching the current filter.
    pub entries: Vec<LibraryEntry>,
    /// Active filter used to produce this snapshot.
    pub filter: LibraryFilter,
    /// Total unfiltered library size.
    pub total_count: usize,
    /// Number of installed entries in the merged library.
    pub installed_count: usize,
    /// Number of metadata-only entries in the merged library.
    pub missing_count: usize,
}

/// Target selection for refresh-style library commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryTarget {
    /// Restrict work to installed entries.
    pub installed_only: bool,
    /// Restrict work to one stable game id.
    pub game_id: Option<GameId>,
    /// Restrict work to one title query.
    pub title: Option<String>,
    /// Restrict work to one ROM path.
    pub rom_path: Option<PathBuf>,
}

/// Summary returned by cache refresh actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshSummary {
    /// Number of metadata records written.
    pub metadata_records: usize,
    /// Number of cover files written.
    pub covers_written: usize,
    /// Number of cheat records written.
    pub cheat_records: usize,
}

/// Provider interface for remote game metadata.
pub trait GameMetadataProvider {
    /// Refresh and return the full metadata index for the current platform.
    fn refresh_metadata(
        &self,
        client: &Client,
        settings: &ProviderSettings,
    ) -> Result<Vec<GameMetadata>>;
}

/// Provider interface for cover downloads.
pub trait CoverProvider {
    /// Download and cache one cover asset if a remote URL is available.
    fn fetch_cover(
        &self,
        client: &Client,
        metadata: &GameMetadata,
        cache_root: &Path,
    ) -> Result<Option<CoverAsset>>;
}

/// Provider interface for cheat downloads.
pub trait CheatProvider {
    /// Refresh and return cached cheat entries for one library entry.
    fn refresh_cheats(
        &self,
        client: &Client,
        settings: &ProviderSettings,
        entry: &LibraryEntry,
        cache_root: &Path,
        enabled_ids: &BTreeSet<String>,
    ) -> Result<Vec<CheatEntry>>;
}

/// Future hook for ROM-download providers. Disabled in v1.
pub trait RomDownloadProvider {
    /// Attempt to download a ROM for the provided game.
    fn download_rom(&self, _entry: &LibraryEntry) -> Result<PathBuf> {
        Err(anyhow!(
            "ROM downloading is intentionally unsupported in this version"
        ))
    }
}

#[derive(Debug, Clone)]
struct LibretroMetadataProvider;

#[derive(Debug, Clone)]
struct LibretroCoverProvider;

#[derive(Debug, Clone)]
struct LibretroCheatProvider;

/// Library/catalog service shared by CLI and GUI shells.
pub struct LibraryService {
    config: RuntimeConfig,
    assets: AssetConfig,
    client: Client,
    metadata_provider: LibretroMetadataProvider,
    cover_provider: LibretroCoverProvider,
    cheat_provider: LibretroCheatProvider,
}

impl std::fmt::Debug for LibraryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryService")
            .field("config", &self.config)
            .field("assets", &self.assets)
            .finish()
    }
}

impl LibraryService {
    /// Build a reusable library service around the provided config and asset paths.
    pub fn new(config: RuntimeConfig, assets: AssetConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent("starbyte/0.1 (+https://github.com/openai/starbyte)")
            .build()
            .context("failed to build library HTTP client")?;
        Ok(Self {
            config,
            assets,
            client,
            metadata_provider: LibretroMetadataProvider,
            cover_provider: LibretroCoverProvider,
            cheat_provider: LibretroCheatProvider,
        })
    }

    /// Borrow the current runtime config.
    #[must_use]
    pub const fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Mutably borrow the current runtime config.
    pub fn config_mut(&mut self) -> &mut RuntimeConfig {
        &mut self.config
    }

    /// Resolve the effective cache root.
    #[must_use]
    pub fn cache_root(&self) -> PathBuf {
        self.config
            .library
            .cache_dir
            .clone()
            .or_else(|| self.assets.cache_dir.clone())
            .unwrap_or_else(|| self.assets.cache_root())
    }

    /// Persist the current config using the active asset/config path rules.
    pub fn save_config(&self) -> Result<()> {
        let path = self.assets.config_path();
        self.config
            .save_to_path(&path)
            .with_context(|| format!("failed to save config to {}", path.display()))
            .map_err(anyhow::Error::from)
    }

    /// Scan configured ROM directories and return installed entries.
    pub fn scan_roms(&self) -> Result<Vec<LocalRomInfo>> {
        let mut discovered = BTreeMap::<GameId, LocalRomInfo>::new();
        for rom_dir in &self.config.library.rom_dirs {
            for path in discover_rom_files(rom_dir)? {
                match inspect_rom_file(&path) {
                    Ok(info) => {
                        discovered.entry(info.game_id.clone()).or_insert(info);
                    }
                    Err(error) => warn!("skipping ROM candidate {}: {error}", path.display()),
                }
            }
        }
        Ok(discovered.into_values().collect())
    }

    /// Load a merged library snapshot using cached metadata, covers, and cheats.
    pub fn snapshot(&self, mut filter: LibraryFilter) -> Result<LibrarySnapshot> {
        if matches!(filter.view_mode, LibraryViewMode::List)
            && !self.config.advanced.show_missing_games
            && !filter.installed_only
        {
            filter.installed_only = false;
        }

        let local_roms = self.scan_roms()?;
        let metadata = self.load_cached_metadata_index()?;
        let enabled_by_game = &self.config.cheats.enabled_by_game;
        let mut entries = merge_library_entries(&local_roms, &metadata, self.cache_root(), enabled_by_game)?;

        let total_count = entries.len();
        let installed_count = entries
            .iter()
            .filter(|entry| entry.installed_status == InstalledStatus::Installed)
            .count();
        let missing_count = total_count.saturating_sub(installed_count);

        if filter.installed_only {
            entries.retain(|entry| entry.installed_status == InstalledStatus::Installed);
        } else if !self.config.advanced.show_missing_games {
            entries.retain(|entry| entry.installed_status == InstalledStatus::Installed);
        }

        if !filter.query.is_empty() {
            let needle = normalize_title(&filter.query);
            entries.retain(|entry| normalize_title(&entry.display_title).contains(&needle));
        }

        entries.sort_by(|left, right| left.display_title.cmp(&right.display_title));

        Ok(LibrarySnapshot {
            entries,
            filter,
            total_count,
            installed_count,
            missing_count,
        })
    }

    /// Refresh the cached metadata index.
    pub fn refresh_metadata_index(&mut self) -> Result<usize> {
        let metadata = self.metadata_provider.refresh_metadata(
            &self.client,
            &self.config.advanced.providers,
        )?;
        write_json(self.metadata_index_path(), &metadata)?;
        self.config.advanced.providers.last_metadata_refresh_unix = Some(now_unix());
        Ok(metadata.len())
    }

    /// Refresh cached cover assets for the targeted entries.
    pub fn refresh_covers(&mut self, target: &LibraryTarget) -> Result<usize> {
        let snapshot = self.snapshot(LibraryFilter {
            installed_only: target.installed_only,
            view_mode: self.config.library.active_view,
            ..LibraryFilter::default()
        })?;
        let mut written = 0;
        for entry in select_entries(snapshot.entries, target) {
            let Some(metadata) = &entry.metadata else {
                continue;
            };
            if self
                .cover_provider
                .fetch_cover(&self.client, metadata, &self.cache_root())?
                .is_some()
            {
                written += 1;
            }
        }
        self.config.advanced.providers.last_cover_refresh_unix = Some(now_unix());
        Ok(written)
    }

    /// Refresh cached cheat files for the targeted entries.
    pub fn refresh_cheats(&mut self, target: &LibraryTarget) -> Result<usize> {
        let snapshot = self.snapshot(LibraryFilter {
            installed_only: target.installed_only,
            view_mode: self.config.library.active_view,
            ..LibraryFilter::default()
        })?;
        let mut records = 0;
        for entry in select_entries(snapshot.entries, target) {
            let enabled_ids = self
                .config
                .cheats
                .enabled_by_game
                .get(&entry.game_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let cheats = self.cheat_provider.refresh_cheats(
                &self.client,
                &self.config.advanced.providers,
                &entry,
                &self.cache_root(),
                &enabled_ids,
            )?;
            records += cheats.len();
        }
        self.config.advanced.providers.last_cheat_refresh_unix = Some(now_unix());
        Ok(records)
    }

    /// Refresh metadata, covers, and cheats for the targeted entries.
    pub fn refresh_all(&mut self, target: &LibraryTarget) -> Result<RefreshSummary> {
        let metadata_records = self.refresh_metadata_index()?;
        let covers_written = self.refresh_covers(target)?;
        let cheat_records = self.refresh_cheats(target)?;
        Ok(RefreshSummary {
            metadata_records,
            covers_written,
            cheat_records,
        })
    }

    fn metadata_index_path(&self) -> PathBuf {
        self.cache_root().join("games").join("metadata").join("index.json")
    }

    fn load_cached_metadata_index(&self) -> Result<Vec<GameMetadata>> {
        read_json_or_default(self.metadata_index_path())
    }
}

impl GameMetadataProvider for LibretroMetadataProvider {
    fn refresh_metadata(
        &self,
        client: &Client,
        settings: &ProviderSettings,
    ) -> Result<Vec<GameMetadata>> {
        if !settings.enable_network {
            return Ok(Vec::new());
        }

        let response = client
            .get(&settings.metadata_index_url)
            .send()
            .context("failed to request metadata index")?
            .error_for_status()
            .context("metadata index request returned an error status")?;
        let tree: GitTreeResponse = response.json().context("failed to parse metadata index response")?;
        let mut metadata = Vec::new();
        for node in tree.tree {
            if !node.path.starts_with("Named_Boxarts/") || node.kind != "blob" {
                continue;
            }
            let Some(title) = Path::new(&node.path).file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let normalized_title = normalize_title(title);
            if normalized_title.is_empty() {
                continue;
            }
            let encoded_path = encode(&node.path);
            metadata.push(GameMetadata {
                game_id: game_id_for_title(title),
                title: title.to_owned(),
                normalized_title,
                source: "libretro-thumbnails".to_owned(),
                cover_url: Some(format!("{}/{}", settings.cover_index_url, encoded_path)),
                has_cheat_files: false,
                fetched_at_unix: now_unix(),
            });
        }
        Ok(metadata)
    }
}

impl CoverProvider for LibretroCoverProvider {
    fn fetch_cover(
        &self,
        client: &Client,
        metadata: &GameMetadata,
        cache_root: &Path,
    ) -> Result<Option<CoverAsset>> {
        let Some(source_url) = &metadata.cover_url else {
            return Ok(None);
        };
        let extension = source_url
            .rsplit('.')
            .next()
            .filter(|ext| ext.len() <= 4)
            .unwrap_or("png");
        let cache_path = cache_root
            .join("games")
            .join("covers")
            .join(format!("{}.{}", metadata.game_id, extension));
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let bytes = client
            .get(source_url)
            .send()
            .with_context(|| format!("failed to request cover from {source_url}"))?
            .error_for_status()
            .with_context(|| format!("cover request returned an error status for {source_url}"))?
            .bytes()
            .context("failed to read cover response bytes")?;
        fs::write(&cache_path, &bytes)?;
        Ok(Some(CoverAsset {
            game_id: metadata.game_id.clone(),
            cache_path,
            source_url: source_url.clone(),
            fetched_at_unix: now_unix(),
        }))
    }
}

impl CheatProvider for LibretroCheatProvider {
    fn refresh_cheats(
        &self,
        client: &Client,
        settings: &ProviderSettings,
        entry: &LibraryEntry,
        cache_root: &Path,
        enabled_ids: &BTreeSet<String>,
    ) -> Result<Vec<CheatEntry>> {
        if !settings.enable_network {
            return Ok(Vec::new());
        }

        let response = client
            .get(&settings.cheat_index_url)
            .send()
            .context("failed to request cheat index")?
            .error_for_status()
            .context("cheat index request returned an error status")?;
        let tree: GitTreeResponse = response.json().context("failed to parse cheat index response")?;
        let mut cheats = Vec::new();
        for node in tree.tree {
            if !node.path.starts_with("cht/Nintendo - Super Nintendo Entertainment System/")
                || !node.path.ends_with(".cht")
                || node.kind != "blob"
            {
                continue;
            }
            let Some(stem) = Path::new(&node.path).file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let base_title = strip_cheat_suffix(stem);
            if normalize_title(&base_title) != normalize_title(&entry.display_title) {
                continue;
            }
            let raw_url = format!(
                "https://raw.githubusercontent.com/libretro/libretro-database/master/{}",
                encode(&node.path)
            );
            let text = client
                .get(&raw_url)
                .send()
                .with_context(|| format!("failed to request cheat file {raw_url}"))?
                .error_for_status()
                .with_context(|| format!("cheat request returned an error status for {raw_url}"))?
                .text()
                .context("failed to read cheat file text")?;
            cheats.extend(parse_libretro_cheats(
                entry.game_id.clone(),
                &text,
                stem,
                enabled_ids,
            ));
        }

        let cache_path = cheat_cache_path(cache_root, &entry.game_id);
        write_json(cache_path, &cheats)?;
        Ok(cheats)
    }
}

#[derive(Debug, Deserialize)]
struct GitTreeResponse {
    tree: Vec<GitTreeNode>,
}

#[derive(Debug, Deserialize)]
struct GitTreeNode {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

fn merge_library_entries(
    local_roms: &[LocalRomInfo],
    metadata_records: &[GameMetadata],
    cache_root: PathBuf,
    enabled_by_game: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<LibraryEntry>> {
    let mut local_by_id = BTreeMap::new();
    for local in local_roms {
        local_by_id.insert(local.game_id.clone(), local.clone());
    }
    let mut metadata_by_id = BTreeMap::new();
    for metadata in metadata_records {
        metadata_by_id.insert(metadata.game_id.clone(), metadata.clone());
    }

    let keys = local_by_id
        .keys()
        .chain(metadata_by_id.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut entries = Vec::new();
    for game_id in keys {
        let local = local_by_id.get(&game_id).cloned();
        let metadata = metadata_by_id.get(&game_id).cloned();
        let display_title = metadata
            .as_ref()
            .map(|record| record.title.clone())
            .or_else(|| local.as_ref().map(|record| record.title.clone()))
            .unwrap_or_else(|| game_id.clone());
        let cover = load_cached_cover(&cache_root, &game_id, metadata.as_ref())?;
        let cheats = load_cached_cheats(
            &cache_root,
            &game_id,
            enabled_by_game.get(&game_id).cloned().unwrap_or_default(),
        )?;
        entries.push(LibraryEntry {
            game_id,
            display_title,
            installed_status: if local.is_some() {
                InstalledStatus::Installed
            } else {
                InstalledStatus::Missing
            },
            local,
            metadata,
            cover,
            cheats,
        });
    }
    Ok(entries)
}

fn load_cached_cover(
    cache_root: &Path,
    game_id: &str,
    metadata: Option<&GameMetadata>,
) -> Result<Option<CoverAsset>> {
    let cover_dir = cache_root.join("games").join("covers");
    if !cover_dir.exists() {
        return Ok(None);
    }
    for entry in fs::read_dir(&cover_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem != game_id {
            continue;
        }
        return Ok(Some(CoverAsset {
            game_id: game_id.to_owned(),
            cache_path: path,
            source_url: metadata
                .and_then(|record| record.cover_url.clone())
                .unwrap_or_default(),
            fetched_at_unix: now_unix(),
        }));
    }
    Ok(None)
}

fn load_cached_cheats(
    cache_root: &Path,
    game_id: &str,
    enabled_ids: Vec<String>,
) -> Result<Vec<CheatEntry>> {
    let mut cheats: Vec<CheatEntry> = read_json_or_default(cheat_cache_path(cache_root, game_id))?;
    let enabled_ids = enabled_ids.into_iter().collect::<BTreeSet<_>>();
    for cheat in &mut cheats {
        cheat.enabled = enabled_ids.contains(&cheat.id);
    }
    Ok(cheats)
}

fn cheat_cache_path(cache_root: &Path, game_id: &str) -> PathBuf {
    cache_root
        .join("games")
        .join("cheats")
        .join(format!("{game_id}.json"))
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn read_json_or_default<T>(path: PathBuf) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }
    let text = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

fn discover_rom_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_rom_path(&path) {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn is_rom_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "sfc" | "smc" | "swc" | "fig")
    )
}

fn inspect_rom_file(path: &Path) -> Result<LocalRomInfo> {
    let cartridge = Cartridge::load(path)
        .with_context(|| format!("failed to inspect ROM at {}", path.display()))?;
    let metadata = fs::metadata(path)?;
    let title = cartridge.header().title.clone();
    Ok(LocalRomInfo {
        game_id: game_id_for_title(&title),
        normalized_title: normalize_title(&title),
        title,
        rom_path: path.to_path_buf(),
        mapper: format!("{:?}", cartridge.mapper()),
        coprocessor: cartridge.coprocessor_kind().map(|kind| kind.to_string()),
        region: match cartridge.header().region {
            Region::Ntsc => "NTSC",
            Region::Pal => "PAL",
            Region::Unknown => "Unknown",
        }
        .to_owned(),
        file_size_bytes: metadata.len(),
    })
}

fn select_entries(entries: Vec<LibraryEntry>, target: &LibraryTarget) -> Vec<LibraryEntry> {
    entries
        .into_iter()
        .filter(|entry| {
            if let Some(game_id) = &target.game_id {
                if &entry.game_id != game_id {
                    return false;
                }
            }
            if let Some(title) = &target.title {
                if !normalize_title(&entry.display_title).contains(&normalize_title(title)) {
                    return false;
                }
            }
            if let Some(rom_path) = &target.rom_path {
                if entry.local.as_ref().map(|local| &local.rom_path) != Some(rom_path) {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn parse_libretro_cheats(
    game_id: GameId,
    text: &str,
    stem: &str,
    enabled_ids: &BTreeSet<String>,
) -> Vec<CheatEntry> {
    let mut descriptions = BTreeMap::<usize, String>::new();
    let mut codes = BTreeMap::<usize, String>::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').to_owned();
        if let Some(index) = key
            .strip_prefix("cheat")
            .and_then(|rest| rest.split('_').next())
            .and_then(|value| value.parse::<usize>().ok())
        {
            if key.ends_with("_desc") {
                descriptions.insert(index, value);
            } else if key.ends_with("_code") {
                codes.insert(index, value);
            }
        }
    }

    descriptions
        .into_iter()
        .filter_map(|(index, name)| {
            let code = codes.get(&index)?.clone();
            let id = format!("{}-{}", game_id, slugify(&format!("{stem}-{index}")));
            Some(CheatEntry {
                id: id.clone(),
                game_id: game_id.clone(),
                name,
                code,
                source: "libretro-database".to_owned(),
                kind: cheat_kind_from_stem(stem),
                enabled: enabled_ids.contains(&id),
            })
        })
        .collect()
}

fn cheat_kind_from_stem(stem: &str) -> String {
    ["Game Genie", "Action Replay", "Pro Action Replay", "Gold Finger"]
        .into_iter()
        .find(|needle| stem.contains(needle))
        .unwrap_or("Unknown")
        .to_owned()
}

fn strip_cheat_suffix(stem: &str) -> String {
    let mut trimmed = stem.to_owned();
    for needle in [
        " (Game Genie)",
        " (Action Replay)",
        " (Pro Action Replay)",
        " (Gold Finger)",
        " (diff)",
    ] {
        if trimmed.ends_with(needle) {
            trimmed.truncate(trimmed.len().saturating_sub(needle.len()));
        }
    }
    trimmed
}

fn slugify(input: &str) -> String {
    normalize_title(input).replace(' ', "-")
}

fn normalize_title(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut prev_space = false;
    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            prev_space = false;
        } else if !prev_space {
            normalized.push(' ');
            prev_space = true;
        }
    }
    normalized.trim().to_owned()
}

fn game_id_for_title(title: &str) -> GameId {
    let normalized = normalize_title(title);
    let mut hasher = Sha1::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use starbyte_core::manifest::RuntimeConfig;

    use super::{
        GameMetadata, InstalledStatus, LibraryFilter, LibraryService, cheat_cache_path, game_id_for_title,
        merge_library_entries, normalize_title, write_json,
    };

    fn synthetic_rom_bytes(title: &[u8; 21]) -> Vec<u8> {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(title);
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom
    }

    #[test]
    fn normalize_title_collapses_punctuation() {
        assert_eq!(
            normalize_title("Mega Man X (USA, Europe)"),
            "mega man x usa europe"
        );
    }

    #[test]
    fn scan_roms_ignores_invalid_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("good.sfc"), synthetic_rom_bytes(b"STARBYTE VALID GAME  ")).unwrap();
        fs::write(dir.path().join("bad.sfc"), b"not a rom").unwrap();
        let mut config = RuntimeConfig::default();
        config.library.rom_dirs.push(dir.path().to_path_buf());
        let service = LibraryService::new(config, Default::default()).unwrap();
        let roms = service.scan_roms().unwrap();
        assert_eq!(roms.len(), 1);
    }

    #[test]
    fn snapshot_merges_installed_and_metadata_only_entries() {
        let dir = tempdir().unwrap();
        let cache_root = dir.path().join(".cache");
        let mut config = RuntimeConfig::default();
        config.library.rom_dirs.push(dir.path().join("roms"));
        config.library.cache_dir = Some(cache_root.clone());
        fs::create_dir_all(config.library.rom_dirs[0].clone()).unwrap();
        fs::write(
            config.library.rom_dirs[0].join("installed.sfc"),
            synthetic_rom_bytes(b"STARBYTE INSTALLED   "),
        )
        .unwrap();

        let installed_id = game_id_for_title("STARBYTE INSTALLED");
        let missing_id = game_id_for_title("Missing Game (USA)");
        write_json(
            cache_root.join("games").join("metadata").join("index.json"),
            &vec![
                GameMetadata {
                    game_id: installed_id,
                    title: "STARBYTE INSTALLED".to_owned(),
                    normalized_title: normalize_title("STARBYTE INSTALLED"),
                    source: "test".to_owned(),
                    cover_url: None,
                    has_cheat_files: false,
                    fetched_at_unix: 0,
                },
                GameMetadata {
                    game_id: missing_id.clone(),
                    title: "Missing Game (USA)".to_owned(),
                    normalized_title: normalize_title("Missing Game (USA)"),
                    source: "test".to_owned(),
                    cover_url: None,
                    has_cheat_files: false,
                    fetched_at_unix: 0,
                },
            ],
        )
        .unwrap();

        let service = LibraryService::new(config, Default::default()).unwrap();
        let snapshot = service.snapshot(LibraryFilter::default()).unwrap();
        assert_eq!(snapshot.total_count, 2);
        assert_eq!(snapshot.installed_count, 1);
        assert!(snapshot.entries.iter().any(|entry| entry.game_id == missing_id && entry.installed_status == InstalledStatus::Missing));
    }

    #[test]
    fn cached_cheats_roundtrip_with_enabled_state() {
        let dir = tempdir().unwrap();
        let game_id = game_id_for_title("STARBYTE INSTALLED");
        let cheat_path = cheat_cache_path(dir.path(), &game_id);
        write_json(
            cheat_path,
            &vec![super::CheatEntry {
                id: "cheat-1".to_owned(),
                game_id: game_id.clone(),
                name: "Infinite lives".to_owned(),
                code: "7E149C09".to_owned(),
                source: "test".to_owned(),
                kind: "Action Replay".to_owned(),
                enabled: false,
            }],
        )
        .unwrap();
        let entries = merge_library_entries(
            &[],
            &[GameMetadata {
                game_id: game_id.clone(),
                title: "STARBYTE INSTALLED".to_owned(),
                normalized_title: normalize_title("STARBYTE INSTALLED"),
                source: "test".to_owned(),
                cover_url: None,
                has_cheat_files: true,
                fetched_at_unix: 0,
            }],
            dir.path().to_path_buf(),
            &std::collections::BTreeMap::from([(
                game_id.clone(),
                vec!["cheat-1".to_owned()],
            )]),
        )
        .unwrap();
        assert_eq!(entries[0].cheats.len(), 1);
        assert!(entries[0].cheats[0].enabled);
    }
}
