// SPDX-License-Identifier: GPL-3.0

//! AutoEQ manager for fetching and caching profiles.

use super::{AutoEQError, AutoEQProfile, AutoEQProfileMetadata, Result};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const INDEX_CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60); // 7 days
const PROFILE_CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

/// Manager for fetching and caching AutoEQ profiles.
pub struct AutoEQManager {
    cache_dir: PathBuf,
    http_client: reqwest::Client,
    memory_cache: HashMap<String, CacheEntry>,
    lru_order: VecDeque<String>,
    index_cache: Option<Vec<AutoEQProfileMetadata>>,
}

struct CacheEntry {
    profile: AutoEQProfile,
    last_accessed: std::time::Instant,
}

impl AutoEQManager {
    /// Create a new AutoEQ manager.
    pub fn new(cache_dir: PathBuf, timeout: Duration) -> Result<Self> {
        let http_client = reqwest::Client::builder().timeout(timeout).build()?;

        // Ensure cache directory exists
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir)?;
        }

        Ok(Self {
            cache_dir,
            http_client,
            memory_cache: HashMap::new(),
            lru_order: VecDeque::new(),
            index_cache: None,
        })
    }

    /// Fetch the AutoEQ index (list of all profiles).
    pub async fn fetch_index(&mut self) -> Result<Vec<AutoEQProfileMetadata>> {
        // Check memory cache first
        if let Some(ref profiles) = self.index_cache {
            return Ok(profiles.clone());
        }

        // Check disk cache
        if let Some(profiles) = self.load_index_from_disk()? {
            self.index_cache = Some(profiles.clone());
            return Ok(profiles);
        }

        // Fetch from network
        let url = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/INDEX.md";
        let response = self.http_client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(AutoEQError::Network(
                response.error_for_status().unwrap_err(),
            ));
        }

        let content = response.text().await?;
        let profiles = super::parser::parse_index(&content)?;

        // Cache in memory
        self.index_cache = Some(profiles.clone());

        // Save to disk cache (ignore errors to not block on I/O issues)
        if let Err(e) = self.save_index_to_disk(&profiles) {
            eprintln!("Warning: Failed to save index to disk cache: {}", e);
        }

        Ok(profiles)
    }

    /// Fetch a specific AutoEQ profile by path.
    pub async fn fetch_profile(&mut self, path: &str) -> Result<AutoEQProfile> {
        // Check memory cache first
        if let Some(entry) = self.memory_cache.get(path) {
            let profile = entry.profile.clone();
            // Update access time and LRU in separate scope
            if let Some(entry) = self.memory_cache.get_mut(path) {
                entry.last_accessed = std::time::Instant::now();
            }
            self.update_lru(path);
            return Ok(profile);
        }

        // Check disk cache
        if let Some(profile) = self.load_profile_from_disk(path)? {
            self.add_to_memory_cache(path.to_string(), profile.clone());
            return Ok(profile);
        }

        // Fetch from network
        let profile = self.fetch_profile_from_network(path).await?;

        // Add to memory cache
        self.add_to_memory_cache(path.to_string(), profile.clone());

        // Save to disk cache (ignore errors to not block on I/O issues)
        if let Err(e) = self.save_profile_to_disk(path, &profile) {
            eprintln!("Warning: Failed to save profile to disk cache: {}", e);
        }

        Ok(profile)
    }

    async fn fetch_profile_from_network(&self, path: &str) -> Result<AutoEQProfile> {
        // URL-decode and construct the profile URL
        let decoded_path = urlencoding::decode(path)
            .map_err(|e| AutoEQError::InvalidFormat(format!("Invalid path encoding: {}", e)))?;

        let path_components: Vec<&str> = decoded_path.split('/').collect();
        if path_components.is_empty() {
            return Err(AutoEQError::InvalidFormat("Empty path".to_string()));
        }

        let headphone_name = path_components.last().unwrap();
        let encoded_path = path_components
            .iter()
            .map(|c| urlencoding::encode(c))
            .collect::<Vec<_>>()
            .join("/");

        let filename = format!("{} FixedBandEQ.txt", headphone_name);
        let encoded_filename = urlencoding::encode(&filename);

        let url = format!(
            "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/{}/{}",
            encoded_path, encoded_filename
        );

        let response = self.http_client.get(&url).send().await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AutoEQError::ProfileNotFound(path.to_string()));
        }

        if !response.status().is_success() {
            return Err(AutoEQError::Network(
                response.error_for_status().unwrap_err(),
            ));
        }

        let content = response.text().await?;
        super::parser::parse_fixed_band_eq(path, &content)
    }

    fn add_to_memory_cache(&mut self, path: String, profile: AutoEQProfile) {
        const MAX_CACHE_SIZE: usize = 20;

        // Evict LRU if at capacity
        if self.memory_cache.len() >= MAX_CACHE_SIZE
            && !self.memory_cache.contains_key(&path)
            && let Some(lru_key) = self.lru_order.pop_front()
        {
            self.memory_cache.remove(&lru_key);
        }

        // Add or update entry
        self.memory_cache.insert(
            path.clone(),
            CacheEntry {
                profile,
                last_accessed: std::time::Instant::now(),
            },
        );

        // Update LRU
        self.lru_order.retain(|k| k != &path);
        self.lru_order.push_back(path);
    }

    fn update_lru(&mut self, path: &str) {
        self.lru_order.retain(|k| k != path);
        self.lru_order.push_back(path.to_string());
    }

    // Disk cache functions

    fn save_index_to_disk(&self, profiles: &[AutoEQProfileMetadata]) -> Result<()> {
        let cache_path = self.cache_dir.join("index.json");
        let cache_data = CachedIndex {
            timestamp: SystemTime::now(),
            profiles: profiles.to_vec(),
        };
        let json = serde_json::to_string_pretty(&cache_data)?;
        std::fs::write(cache_path, json)?;
        Ok(())
    }

    fn load_index_from_disk(&self) -> Result<Option<Vec<AutoEQProfileMetadata>>> {
        let cache_path = self.cache_dir.join("index.json");
        if !cache_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(cache_path)?;
        let cache_data: CachedIndex = serde_json::from_str(&content)?;

        // Check TTL
        let age = SystemTime::now()
            .duration_since(cache_data.timestamp)
            .unwrap_or(Duration::MAX);

        if age > INDEX_CACHE_TTL {
            return Ok(None);
        }

        Ok(Some(cache_data.profiles))
    }

    fn save_profile_to_disk(&self, path: &str, profile: &AutoEQProfile) -> Result<()> {
        let filename = self.cache_filename(path);
        let cache_path = self.cache_dir.join(filename);

        let cache_data = CachedProfile {
            timestamp: SystemTime::now(),
            profile: profile.clone(),
        };

        let json = serde_json::to_string_pretty(&cache_data)?;
        std::fs::write(cache_path, json)?;
        Ok(())
    }

    fn load_profile_from_disk(&self, path: &str) -> Result<Option<AutoEQProfile>> {
        let filename = self.cache_filename(path);
        let cache_path = self.cache_dir.join(filename);

        if !cache_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(cache_path)?;
        let cache_data: CachedProfile = serde_json::from_str(&content)?;

        // Check TTL
        let age = SystemTime::now()
            .duration_since(cache_data.timestamp)
            .unwrap_or(Duration::MAX);

        if age > PROFILE_CACHE_TTL {
            return Ok(None);
        }

        Ok(Some(cache_data.profile))
    }

    fn cache_filename(&self, path: &str) -> String {
        let hash = format!("{:x}", md5::compute(path.as_bytes()));
        format!("{}.json", hash)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedIndex {
    timestamp: SystemTime,
    profiles: Vec<AutoEQProfileMetadata>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedProfile {
    timestamp: SystemTime,
    profile: AutoEQProfile,
}
