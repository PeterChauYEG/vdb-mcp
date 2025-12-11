use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ============================================================================
// Constants
// ============================================================================

const ALWAYS_IGNORE_DIRS: &[&str] = &[
    ".git",
    ".yarn",
    "assets",
    "docs",
    "cypress",
    "storybook",
    "__mocks__",
    ".maestro",
    ".github",
    "examples",
    "codemods",
    "msw",
    "fastlane",
    "code-signing",
    ".reassure",
    ".vscode",
    ".claude",
    "build",
    "Pods",
    ".gradle",
    "node_modules",
    "dist",
    "coverage",
    ".next",
    ".cache",
    "tmp",
    "temp",
    "target",
    "test-utils",
    "__fixture__",
    "Locales",
    "translations",
    "generated",
    "cache",
    "logs",
];

const BINARY_EXTENSIONS: &[&str] = &[
    // Images
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg", ".webp",
    // Archives
    ".zip", ".tar", ".gz", ".bz2", ".7z", ".rar", ".xz",
    // Compiled/binary
    ".exe", ".dll", ".so", ".dylib", ".a", ".o", ".obj", ".bin",
    // Rust compiled
    ".rmeta", ".rlib",
    // Perl XS compiled objects
    ".os", ".bs",
    // Fonts
    ".ttf", ".otf", ".woff", ".woff2", ".eot",
    // Media
    ".mp3", ".mp4", ".avi", ".mov", ".wav", ".flac", ".ogg",
    // Documents
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
    // Database/data
    ".db", ".sqlite", ".sql",
    // Python compiled
    ".pyc", ".pyo",
    // Java compiled
    ".class", ".jar", ".war",
    // Other binary/generated
    ".onnx", ".ort", ".pck", ".tscn",
    // Lock files
    ".lock",
    // Translations
    ".po", ".mo",
];

// Extensions for generated/non-code files to skip
const GENERATED_EXTENSIONS: &[&str] = &[
    ".map",        // Source maps
    ".d",          // Dependency files
    ".timestamp",  // Build timestamps
    ".min.js",     // Minified JS
    ".min.css",    // Minified CSS
    ".d.ts",       // TypeScript declarations (often generated)
];

const ALWAYS_IGNORE_FILES: &[&str] = &[
    ".DS_Store",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    ".eslintrc",
    ".prettierrc",
    ".npmignore",
    ".gitignore",
];

const ALLOWED_NO_EXTENSION: &[&str] = &["Makefile", "Dockerfile", "Gemfile", "Rakefile", "Podfile", "Containerfile"];

// ============================================================================
// File Utilities
// ============================================================================

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn should_index_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let file_name_lower = file_name.to_lowercase();

    // Check ignored files
    if ALWAYS_IGNORE_FILES.iter().any(|f| file_name_lower == f.to_lowercase()) {
        return false;
    }

    // Check binary extensions
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{}", ext.to_lowercase());
        if BINARY_EXTENSIONS.contains(&ext_with_dot.as_str()) {
            return false;
        }
    }

    // Check generated file patterns (e.g., .d.ts, .min.js)
    for pattern in GENERATED_EXTENSIONS {
        if file_name_lower.ends_with(pattern) {
            return false;
        }
    }

    // Skip test files
    if file_name.contains(".test.") || file_name.contains(".spec.") {
        return false;
    }

    // Skip __tests__ directories
    if path.components().any(|c| c.as_os_str() == "__tests__") {
        return false;
    }

    // Skip files without extension unless they're known config files
    if path.extension().is_none() && !ALLOWED_NO_EXTENSION.contains(&file_name) {
        return false;
    }

    true
}

fn load_gitignore(directory: &Path) -> Option<Gitignore> {
    let gitignore_path = directory.join(".gitignore");
    if gitignore_path.exists() {
        let mut builder = GitignoreBuilder::new(directory);
        if builder.add(&gitignore_path).is_none() {
            return builder.build().ok();
        }
    }
    None
}

fn print_file_audit(files: &[PathBuf], base_dir: &Path) {
    // Collect directories (relative, up to 2 levels deep)
    let mut dir_counts: BTreeMap<String, usize> = BTreeMap::new();
    // Collect extensions
    let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();

    for file in files {
        // Get relative path
        let rel_path = file.strip_prefix(base_dir).unwrap_or(file);

        // Count top-level directories (1-2 levels)
        let components: Vec<_> = rel_path.components().collect();
        if components.len() > 1 {
            let top_dir = components[0].as_os_str().to_string_lossy().to_string();
            *dir_counts.entry(top_dir.clone()).or_insert(0) += 1;

            // Also count 2-level deep for more detail
            if components.len() > 2 {
                let two_level = format!("{}/{}", top_dir, components[1].as_os_str().to_string_lossy());
                *dir_counts.entry(two_level).or_insert(0) += 1;
            }
        } else {
            *dir_counts.entry(".".to_string()).or_insert(0) += 1;
        }

        // Count extensions
        let ext = file.extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_else(|| "(no ext)".to_string());
        *ext_counts.entry(ext).or_insert(0) += 1;
    }

    println!("\n=== File Audit ===");

    // Print top directories (sorted by count, descending)
    println!("\nDirectories (file count):");
    let mut dir_vec: Vec<_> = dir_counts.into_iter().collect();
    dir_vec.sort_by(|a, b| b.1.cmp(&a.1));
    for (dir, count) in dir_vec.iter().take(30) {
        println!("  {:6}  {}", count, dir);
    }
    if dir_vec.len() > 30 {
        println!("  ... and {} more directories", dir_vec.len() - 30);
    }

    // Print extensions (sorted by count, descending)
    println!("\nExtensions (file count):");
    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    for (ext, count) in ext_vec.iter().take(20) {
        println!("  {:6}  {}", count, ext);
    }
    if ext_vec.len() > 20 {
        println!("  ... and {} more extensions", ext_vec.len() - 20);
    }

    println!();
}

// ============================================================================
// TEI Embedding Client (Text Embeddings Inference)
// ============================================================================

pub struct EmbeddingClient {
    client: Client,
    base_url: String,
}

#[derive(Serialize)]
struct TEIRequest {
    inputs: Vec<String>,
}

impl EmbeddingClient {
    pub fn new(tei_url: &str) -> Result<Self> {
        println!("Connecting to TEI embedding service at {}...", tei_url);

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        // Wait for TEI to be ready
        let health_url = format!("{}/health", tei_url);
        for i in 0..30 {
            match client.get(&health_url).send() {
                Ok(resp) if resp.status().is_success() => {
                    println!("  TEI service ready!");
                    return Ok(Self {
                        client,
                        base_url: tei_url.to_string(),
                    });
                }
                _ => {
                    if i < 29 {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                }
            }
        }

        anyhow::bail!("TEI service not available at {}", tei_url)
    }

    pub fn encode(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = TEIRequest {
            inputs: texts.iter().map(|s| s.to_string()).collect(),
        };

        let url = format!("{}/embed", self.base_url);
        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .context("Failed to send request to TEI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            anyhow::bail!("TEI request failed: {} - {}", status, body);
        }

        let embeddings: Vec<Vec<f32>> = response.json()
            .context("Failed to parse TEI response")?;

        Ok(embeddings)
    }
}

// ============================================================================
// Chunking
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ChunkMetadata {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_type: String,
    pub content_hash: String,
    pub file_hash: String,
    pub git_commit: String,
    pub git_branch: String,
    pub indexed_at: u64,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: String,
    pub text: String,
    pub metadata: ChunkMetadata,
}

pub struct CodeChunker {
    git_commit: String,
    git_branch: String,
}

impl CodeChunker {
    pub fn new(git_commit: String, git_branch: String) -> Self {
        Self { git_commit, git_branch }
    }

    pub fn chunk_code(&self, content: &str, file_path: &str, file_hash: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut current_chunk: Vec<&str> = Vec::new();
        let mut current_size = 0usize;
        let mut start_line = 1usize;

        for (i, line) in lines.iter().enumerate() {
            let line_size = line.len() + 1;

            if current_size + line_size > chunk_size && !current_chunk.is_empty() {
                let chunk_text = current_chunk.join("\n");
                let end_line = start_line + current_chunk.len() - 1;
                chunks.push(self.create_chunk(file_path, &chunk_text, file_hash, start_line, end_line));

                let overlap_lines = self.get_overlap_lines(&current_chunk, overlap);
                let overlap_count = overlap_lines.len();
                current_chunk = overlap_lines;
                current_size = current_chunk.iter().map(|l| l.len() + 1).sum();
                start_line = i + 1 - overlap_count;
            }

            current_chunk.push(line);
            current_size += line_size;
        }

        if !current_chunk.is_empty() {
            let chunk_text = current_chunk.join("\n");
            let end_line = start_line + current_chunk.len() - 1;
            chunks.push(self.create_chunk(file_path, &chunk_text, file_hash, start_line, end_line));
        }

        chunks
    }

    fn create_chunk(&self, file_path: &str, chunk_text: &str, file_hash: &str, start_line: usize, end_line: usize) -> Chunk {
        let file_type = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();

        let commit_prefix = if self.git_commit.len() >= 8 { &self.git_commit[..8] } else { &self.git_commit };
        let id = format!("{}_{}_{}_{}_{}",
            self.git_branch, commit_prefix,
            file_path.replace('/', "_").replace('.', "_"),
            start_line, end_line
        );

        Chunk {
            id,
            text: chunk_text.to_string(),
            metadata: ChunkMetadata {
                file_path: file_path.to_string(),
                start_line,
                end_line,
                file_type,
                content_hash: hash_content(chunk_text),
                file_hash: file_hash.to_string(),
                git_commit: self.git_commit.clone(),
                git_branch: self.git_branch.clone(),
                indexed_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            },
        }
    }

    fn get_overlap_lines<'a>(&self, current_chunk: &[&'a str], overlap: usize) -> Vec<&'a str> {
        let mut overlap_lines = Vec::new();
        let mut overlap_size = 0usize;
        for line in current_chunk.iter().rev() {
            let line_size = line.len() + 1;
            if overlap_size + line_size > overlap { break; }
            overlap_lines.insert(0, *line);
            overlap_size += line_size;
        }
        overlap_lines
    }
}

// ============================================================================
// ChromaDB Client
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct ChromaCollection {
    id: String,
    name: String,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ChromaAddRequest {
    ids: Vec<String>,
    embeddings: Vec<Vec<f32>>,
    documents: Vec<String>,
    metadatas: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ChromaDeleteRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#where: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ChromaGetRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#where: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
    include: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ChromaGetResponse {
    ids: Vec<String>,
    #[serde(default)]
    metadatas: Option<Vec<serde_json::Value>>,
}

pub struct ChromaClient {
    client: Client,
    base_url: String,
    collection_id: Option<String>,
    collection_name: String,
}

impl ChromaClient {
    pub fn new(host: &str, port: &str, collection_name: &str) -> Result<Self> {
        let client = Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
        let base_url = format!("http://{}:{}/api/v2/tenants/default_tenant/databases/default_database", host, port);

        let mut chroma = Self {
            client,
            base_url,
            collection_id: None,
            collection_name: collection_name.to_string(),
        };

        chroma.get_or_create_collection()?;
        Ok(chroma)
    }

    fn get_or_create_collection(&mut self) -> Result<()> {
        let url = format!("{}/collections", self.base_url);
        let response = self.client.get(&url).send();

        if let Ok(resp) = response {
            if resp.status().is_success() {
                let collections: Vec<ChromaCollection> = resp.json().unwrap_or_default();
                for collection in collections {
                    if collection.name == self.collection_name {
                        self.collection_id = Some(collection.id);
                        println!("Using existing collection: {}", self.collection_name);
                        return Ok(());
                    }
                }
            }
        }

        let body = serde_json::json!({
            "name": self.collection_name,
            "metadata": { "description": "Codebase index for vector MCP", "hnsw:space": "cosine" }
        });

        let response = self.client.post(&url).json(&body).send().context("Failed to create collection")?;

        if response.status().is_success() {
            let collection: ChromaCollection = response.json()?;
            self.collection_id = Some(collection.id);
            println!("Created new collection: {}", self.collection_name);
        } else {
            anyhow::bail!("Failed to create collection: {}", response.text().unwrap_or_default());
        }

        Ok(())
    }

    pub fn add_chunks(&self, chunks: &[Chunk], embeddings: Vec<Vec<f32>>) -> Result<()> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/add", self.base_url, collection_id);

        let request = ChromaAddRequest {
            ids: chunks.iter().map(|c| c.id.clone()).collect(),
            embeddings,
            documents: chunks.iter().map(|c| c.text.clone()).collect(),
            metadatas: chunks.iter().map(|c| serde_json::to_value(&c.metadata).unwrap()).collect(),
        };

        let response = self.client.post(&url).json(&request).send().context("Failed to add chunks")?;

        if !response.status().is_success() {
            let error_text = response.text().unwrap_or_default();
            if !error_text.contains("already exists") && !error_text.contains("Duplicate") {
                anyhow::bail!("Failed to add chunks: {}", error_text);
            }
        }

        Ok(())
    }

    pub fn get_indexed_files(&self) -> Result<HashMap<String, IndexedFileInfo>> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let mut indexed_files = HashMap::new();
        let mut offset = 0;
        let limit = 1000;

        loop {
            let url = format!("{}/collections/{}/get", self.base_url, collection_id);
            let request = ChromaGetRequest {
                ids: None, r#where: None, limit: Some(limit), offset: Some(offset),
                include: vec!["metadatas".to_string()],
            };

            let response = self.client.post(&url).json(&request).send().context("Failed to get indexed files")?;
            if !response.status().is_success() { break; }

            let get_response: ChromaGetResponse = response.json()?;
            if get_response.ids.is_empty() { break; }

            if let Some(metadatas) = get_response.metadatas {
                for metadata in metadatas {
                    if let (Some(file_path), Some(git_commit), Some(file_hash)) = (
                        metadata.get("file_path").and_then(|v| v.as_str()),
                        metadata.get("git_commit").and_then(|v| v.as_str()),
                        metadata.get("file_hash").and_then(|v| v.as_str()),
                    ) {
                        indexed_files.insert(file_path.to_string(), IndexedFileInfo {
                            git_commit: git_commit.to_string(),
                            file_hash: file_hash.to_string(),
                        });
                    }
                }
            }

            offset += limit;
            if get_response.ids.len() < limit { break; }
        }

        Ok(indexed_files)
    }

    pub fn check_branch_indexed(&self, git_branch: &str, git_commit: &str) -> Result<bool> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/get", self.base_url, collection_id);

        let request = ChromaGetRequest {
            ids: None,
            r#where: Some(serde_json::json!({
                "$and": [{"git_branch": {"$eq": git_branch}}, {"git_commit": {"$eq": git_commit}}]
            })),
            limit: Some(1), offset: None,
            include: vec!["metadatas".to_string()],
        };

        if let Ok(resp) = self.client.post(&url).json(&request).send() {
            if resp.status().is_success() {
                let get_response: ChromaGetResponse = resp.json()?;
                return Ok(!get_response.ids.is_empty());
            }
        }
        Ok(false)
    }

    pub fn cleanup_old_branch_commits(&self, git_branch: &str, current_commit: &str) -> Result<()> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/get", self.base_url, collection_id);

        let request = ChromaGetRequest {
            ids: None,
            r#where: Some(serde_json::json!({
                "$and": [{"git_branch": {"$eq": git_branch}}, {"git_commit": {"$ne": current_commit}}]
            })),
            limit: Some(10000), offset: None,
            include: vec!["metadatas".to_string()],
        };

        let response = self.client.post(&url).json(&request).send()?;
        if response.status().is_success() {
            let get_response: ChromaGetResponse = response.json()?;
            if !get_response.ids.is_empty() {
                println!("Cleaning up {} old chunks from branch {}", get_response.ids.len(), git_branch);
                for chunk_ids in get_response.ids.chunks(1000) {
                    let delete_url = format!("{}/collections/{}/delete", self.base_url, collection_id);
                    let delete_request = ChromaDeleteRequest { ids: Some(chunk_ids.to_vec()), r#where: None };
                    self.client.post(&delete_url).json(&delete_request).send()?;
                }
            }
        }
        Ok(())
    }

    pub fn delete_file_chunks(&self, file_path: &str) -> Result<()> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/delete", self.base_url, collection_id);
        let request = ChromaDeleteRequest {
            ids: None,
            r#where: Some(serde_json::json!({"file_path": {"$eq": file_path}})),
        };
        self.client.post(&url).json(&request).send()?;
        Ok(())
    }

    pub fn get_collection_count(&self) -> Result<usize> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/count", self.base_url, collection_id);
        let response = self.client.get(&url).send()?;
        if response.status().is_success() {
            return Ok(response.json()?);
        }
        Ok(0)
    }
}

#[derive(Debug, Clone)]
pub struct IndexedFileInfo {
    pub git_commit: String,
    pub file_hash: String,
}

// ============================================================================
// Indexer
// ============================================================================

pub struct CodebaseIndexer {
    chroma: ChromaClient,
    embedding_client: EmbeddingClient,
    git_hash: String,
    git_branch: String,
    chunker: CodeChunker,
}

impl CodebaseIndexer {
    pub fn new(chroma_host: &str, chroma_port: &str, collection_name: &str, tei_url: &str, git_hash: String, git_branch: String) -> Result<Self> {
        println!("Connecting to ChromaDB at {}:{}...", chroma_host, chroma_port);
        let chroma = ChromaClient::new(chroma_host, chroma_port, collection_name)?;

        let embedding_client = EmbeddingClient::new(tei_url)?;
        let chunker = CodeChunker::new(git_hash.clone(), git_branch.clone());

        Ok(Self { chroma, embedding_client, git_hash, git_branch, chunker })
    }

    pub fn index_directory(&self, directory: &Path, batch_size: usize, incremental: bool, max_file_size_mb: usize) -> Result<()> {
        println!("Indexing {}...", directory.display());

        if incremental && !self.git_hash.is_empty() && !self.git_branch.is_empty() {
            if self.chroma.check_branch_indexed(&self.git_branch, &self.git_hash)? {
                println!("Branch {} at commit {} is already indexed, skipping.",
                    self.git_branch, &self.git_hash[..self.git_hash.len().min(8)]);
                self.print_stats()?;
                return Ok(());
            }
            self.chroma.cleanup_old_branch_commits(&self.git_branch, &self.git_hash)?;
        }

        let indexed_files = if incremental {
            self.chroma.get_indexed_files().unwrap_or_default()
        } else {
            HashMap::new()
        };

        let (all_files, files_to_index) = self.scan_directory(directory, &indexed_files, max_file_size_mb)?;

        // Print audit summary
        print_file_audit(&all_files, directory);

        println!("Found {} total files", all_files.len());
        println!("Processing {} files", files_to_index.len());

        if files_to_index.is_empty() {
            println!("No files to index.");
            self.print_stats()?;
            return Ok(());
        }

        if incremental {
            for file_path in &files_to_index {
                let relative_path = file_path.strip_prefix(directory).unwrap_or(file_path).to_string_lossy();
                if indexed_files.contains_key(relative_path.as_ref()) {
                    let _ = self.chroma.delete_file_chunks(&relative_path);
                }
            }
        }

        self.process_files_parallel(directory, &files_to_index, batch_size, max_file_size_mb)?;
        self.print_stats()?;
        Ok(())
    }

    fn scan_directory(&self, directory: &Path, indexed_files: &HashMap<String, IndexedFileInfo>, max_file_size_mb: usize) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        println!("Scanning codebase...");
        let gitignore = load_gitignore(directory);
        let mut all_files = Vec::new();
        let mut files_to_index = Vec::new();
        let ignore_dirs: HashSet<&str> = ALWAYS_IGNORE_DIRS.iter().cloned().collect();

        for entry in walkdir::WalkDir::new(directory)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let path = e.path();
                let is_dir = e.file_type().is_dir();

                // Check hardcoded ignore dirs
                if is_dir {
                    let dir_name = e.file_name().to_str().unwrap_or("");
                    if ignore_dirs.contains(dir_name) {
                        return false;
                    }
                }

                // Check gitignore for both files and directories
                if let Some(ref gi) = gitignore {
                    if gi.matched(path, is_dir).is_ignore() {
                        return false;
                    }
                }

                true
            })
        {
            let entry = match entry { Ok(e) => e, Err(_) => continue };
            if !entry.file_type().is_file() { continue; }

            let path = entry.path();
            if !should_index_file(path) { continue; }
            if let Ok(metadata) = path.metadata() {
                if metadata.len() > (max_file_size_mb * 1024 * 1024) as u64 { continue; }
            }

            all_files.push(path.to_path_buf());

            let relative_path = path.strip_prefix(directory).unwrap_or(path).to_string_lossy().to_string();
            let needs_reindex = if let Some(info) = indexed_files.get(&relative_path) {
                info.git_commit != self.git_hash
            } else {
                true
            };

            if needs_reindex {
                files_to_index.push(path.to_path_buf());
            }
        }

        Ok((all_files, files_to_index))
    }

    fn process_files_parallel(&self, base_directory: &Path, files: &[PathBuf], batch_size: usize, max_file_size_mb: usize) -> Result<()> {
        let processed_count = Arc::new(Mutex::new(0usize));
        let total_files = files.len();

        let all_chunks: Vec<Chunk> = files
            .par_iter()
            .filter_map(|file_path| {
                match self.process_single_file(base_directory, file_path, max_file_size_mb) {
                    Ok(chunks) => {
                        let mut count = processed_count.lock().unwrap();
                        *count += 1;
                        if *count % 100 == 0 {
                            println!("Processed {}/{} files", *count, total_files);
                        }
                        Some(chunks)
                    }
                    Err(e) => {
                        eprintln!("Error processing {}: {}", file_path.display(), e);
                        None
                    }
                }
            })
            .flatten()
            .collect();

        println!("Generated {} chunks from {} files", all_chunks.len(), total_files);

        for (batch_idx, chunk_batch) in all_chunks.chunks(batch_size).enumerate() {
            println!("Embedding and uploading batch {}/{}...", batch_idx + 1, (all_chunks.len() + batch_size - 1) / batch_size);

            let texts: Vec<&str> = chunk_batch.iter().map(|c| c.text.as_str()).collect();
            let embeddings = self.embedding_client.encode(&texts)?;
            self.chroma.add_chunks(chunk_batch, embeddings)?;
        }

        println!("Indexing complete!");
        Ok(())
    }

    fn process_single_file(&self, base_directory: &Path, file_path: &Path, max_file_size_mb: usize) -> Result<Vec<Chunk>> {
        let metadata = fs::metadata(file_path)?;
        if metadata.len() > (max_file_size_mb * 1024 * 1024) as u64 {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(file_path).unwrap_or_default();
        if content.is_empty() { return Ok(Vec::new()); }

        let file_hash = hash_content(&content);
        let relative_path = file_path.strip_prefix(base_directory).unwrap_or(file_path).to_string_lossy().to_string();
        let chunks = self.chunker.chunk_code(&content, &relative_path, &file_hash, 3000, 500);

        Ok(chunks)
    }

    fn print_stats(&self) -> Result<()> {
        let count = self.chroma.get_collection_count()?;
        println!("\n=== Collection Stats ===");
        println!("Total chunks: {}", count);
        if !self.git_branch.is_empty() { println!("Branch: {}", self.git_branch); }
        if !self.git_hash.is_empty() { println!("Commit: {}", &self.git_hash[..self.git_hash.len().min(8)]); }
        Ok(())
    }
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "indexer")]
#[command(about = "Index codebase and populate vector database for MCP server usage")]
struct IndexerArgs {
    #[arg(long)]
    directory: String,
    #[arg(long, default_value = "localhost")]
    host: String,
    #[arg(long, default_value = "8000")]
    port: String,
    #[arg(long, default_value = "codebase")]
    collection: String,
    #[arg(long, default_value_t = 64)]
    batch_size: usize,
    #[arg(long, default_value_t = false)]
    no_incremental: bool,
    #[arg(long, default_value_t = 10)]
    max_file_size: usize,
}

fn main() -> Result<()> {
    let args = IndexerArgs::parse();
    let directory = PathBuf::from(&args.directory);
    if !directory.is_dir() {
        anyhow::bail!("{} is not a directory", args.directory);
    }

    let git_hash = env::var("GIT_HASH").unwrap_or_default();
    let git_branch = env::var("GIT_BRANCH").unwrap_or_default();
    let tei_url = env::var("TEI_URL").unwrap_or_else(|_| "http://localhost:8081".to_string());

    println!("=== Rust Codebase Indexer ===");
    println!("Directory: {}", args.directory);
    println!("ChromaDB: {}:{}", args.host, args.port);
    println!("TEI: {}", tei_url);
    println!("Collection: {}", args.collection);
    println!("Batch size: {}", args.batch_size);
    println!("Max file size: {} MB", args.max_file_size);
    println!("Incremental: {}", !args.no_incremental);
    if !git_branch.is_empty() { println!("Git branch: {}", git_branch); }
    if !git_hash.is_empty() { println!("Git commit: {}", &git_hash[..git_hash.len().min(8)]); }
    println!();

    let indexer = CodebaseIndexer::new(&args.host, &args.port, &args.collection, &tei_url, git_hash, git_branch)?;
    indexer.index_directory(&directory, args.batch_size, !args.no_incremental, args.max_file_size)?;

    Ok(())
}
