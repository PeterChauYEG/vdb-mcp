use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use clap::Parser;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

// ============================================================================
// Constants
// ============================================================================

const ALWAYS_IGNORE_DIRS: &[&str] = &[
    ".git", ".yarn", "assets", "docs", "cypress", "storybook", "__mocks__",
    ".maestro", ".github", "examples", "codemods", "msw", "fastlane",
    "code-signing", ".reassure", ".vscode", ".claude", "build", "Pods",
    ".gradle", "node_modules", "dist", "coverage", ".next", ".cache",
    "tmp", "temp", "target", "test-utils", "__fixture__", "Locales",
    "translations", "generated", "cache", "logs",
];

const BINARY_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg", ".webp",
    ".zip", ".tar", ".gz", ".bz2", ".7z", ".rar", ".xz",
    ".exe", ".dll", ".so", ".dylib", ".a", ".o", ".obj", ".bin",
    ".rmeta", ".rlib", ".os", ".bs",
    ".ttf", ".otf", ".woff", ".woff2", ".eot",
    ".mp3", ".mp4", ".avi", ".mov", ".wav", ".flac", ".ogg",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
    ".db", ".sqlite", ".sql", ".pyc", ".pyo", ".class", ".jar", ".war",
    ".onnx", ".ort", ".pck", ".tscn", ".lock", ".po", ".mo",
];

const GENERATED_EXTENSIONS: &[&str] = &[".map", ".d", ".timestamp", ".min.js", ".min.css", ".d.ts"];

const ALWAYS_IGNORE_FILES: &[&str] = &[
    ".DS_Store", "package-lock.json", "yarn.lock", "pnpm-lock.yaml",
    "Cargo.lock", ".eslintrc", ".prettierrc", ".npmignore", ".gitignore",
];

const ALLOWED_NO_EXTENSION: &[&str] = &["Makefile", "Dockerfile", "Gemfile", "Rakefile", "Podfile", "Containerfile"];

// ============================================================================
// File Utilities
// ============================================================================

fn should_index_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let file_name_lower = file_name.to_lowercase();

    if ALWAYS_IGNORE_FILES.iter().any(|f| file_name_lower == f.to_lowercase()) {
        return false;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{}", ext.to_lowercase());
        if BINARY_EXTENSIONS.contains(&ext_with_dot.as_str()) {
            return false;
        }
    }

    for pattern in GENERATED_EXTENSIONS {
        if file_name_lower.ends_with(pattern) {
            return false;
        }
    }

    if file_name.contains(".test.") || file_name.contains(".spec.") {
        return false;
    }

    if path.components().any(|c| c.as_os_str() == "__tests__") {
        return false;
    }

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

// ============================================================================
// Embedding Client
// ============================================================================

pub struct EmbeddingClient {
    client: Client,
    base_url: String,
}

#[derive(Serialize)]
struct EmbedRequest {
    inputs: Vec<String>,
}

impl EmbeddingClient {
    pub fn new(url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let health_url = format!("{}/health", url);
        for _ in 0..30 {
            if let Ok(resp) = client.get(&health_url).send() {
                if resp.status().is_success() {
                    return Ok(Self { client, base_url: url.to_string() });
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        anyhow::bail!("Embedding service not available at {}", url)
    }

    pub fn encode(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbedRequest {
            inputs: texts.iter().map(|s| s.to_string()).collect(),
        };

        let response = self.client
            .post(&format!("{}/embed", self.base_url))
            .json(&request)
            .send()
            .context("Failed to send embedding request")?;

        if !response.status().is_success() {
            anyhow::bail!("Embedding request failed: {}", response.status());
        }

        Ok(response.json()?)
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
    pub git_commit: String,
    pub git_branch: String,
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

    pub fn chunk_code(&self, content: &str, file_path: &str) -> Vec<Chunk> {
        let chunk_size = 3000;
        let overlap = 500;
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut current_chunk: Vec<&str> = Vec::new();
        let mut current_size = 0usize;
        let mut start_line = 1usize;

        for (i, line) in lines.iter().enumerate() {
            let line_size = line.len() + 1;

            if current_size + line_size > chunk_size && !current_chunk.is_empty() {
                chunks.push(self.create_chunk(file_path, &current_chunk, start_line));

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
            chunks.push(self.create_chunk(file_path, &current_chunk, start_line));
        }

        chunks
    }

    fn create_chunk(&self, file_path: &str, lines: &[&str], start_line: usize) -> Chunk {
        let end_line = start_line + lines.len() - 1;
        let chunk_text = lines.join("\n");

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
            text: chunk_text,
            metadata: ChunkMetadata {
                file_path: file_path.to_string(),
                start_line,
                end_line,
                file_type,
                git_commit: self.git_commit.clone(),
                git_branch: self.git_branch.clone(),
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
}

#[derive(Debug, Serialize)]
struct ChromaAddRequest {
    ids: Vec<String>,
    embeddings: Vec<Vec<f32>>,
    metadatas: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ChromaQueryRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    r#where: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    include: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ChromaQueryResponse {
    ids: Vec<String>,
}

#[derive(Clone)]
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

        if let Ok(resp) = self.client.get(&url).send() {
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
            "metadata": { "hnsw:space": "cosine" }
        });

        let response = self.client.post(&url).json(&body).send()?;
        if response.status().is_success() {
            let collection: ChromaCollection = response.json()?;
            self.collection_id = Some(collection.id);
            println!("Created new collection: {}", self.collection_name);
        } else {
            anyhow::bail!("Failed to create collection");
        }

        Ok(())
    }

    pub fn add_chunks(&self, chunks: &[Chunk], embeddings: Vec<Vec<f32>>) -> Result<()> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/add", self.base_url, collection_id);

        let request = ChromaAddRequest {
            ids: chunks.iter().map(|c| c.id.clone()).collect(),
            embeddings,
            metadatas: chunks.iter().map(|c| serde_json::to_value(&c.metadata).unwrap()).collect(),
        };

        let response = self.client.post(&url).json(&request).send()?;
        if !response.status().is_success() {
            let error_text = response.text().unwrap_or_default();
            if !error_text.contains("already exists") && !error_text.contains("Duplicate") {
                anyhow::bail!("Failed to add chunks: {}", error_text);
            }
        }

        Ok(())
    }

    pub fn is_commit_indexed(&self, git_branch: &str, git_commit: &str) -> bool {
        let Some(collection_id) = &self.collection_id else { return false };
        let url = format!("{}/collections/{}/get", self.base_url, collection_id);

        let request = ChromaQueryRequest {
            r#where: Some(serde_json::json!({
                "$and": [{"git_branch": {"$eq": git_branch}}, {"git_commit": {"$eq": git_commit}}]
            })),
            limit: Some(1),
            include: vec![],
        };

        if let Ok(resp) = self.client.post(&url).json(&request).send() {
            if let Ok(result) = resp.json::<ChromaQueryResponse>() {
                return !result.ids.is_empty();
            }
        }
        false
    }

    pub fn delete_old_commits(&self, git_branch: &str, current_commit: &str) -> Result<usize> {
        let collection_id = self.collection_id.as_ref().context("Collection not initialized")?;
        let url = format!("{}/collections/{}/get", self.base_url, collection_id);

        let request = ChromaQueryRequest {
            r#where: Some(serde_json::json!({
                "$and": [{"git_branch": {"$eq": git_branch}}, {"git_commit": {"$ne": current_commit}}]
            })),
            limit: Some(50000),
            include: vec![],
        };

        let response = self.client.post(&url).json(&request).send()?;
        if !response.status().is_success() {
            return Ok(0);
        }

        let result: ChromaQueryResponse = response.json()?;
        if result.ids.is_empty() {
            return Ok(0);
        }

        let count = result.ids.len();
        for chunk_ids in result.ids.chunks(1000) {
            let delete_url = format!("{}/collections/{}/delete", self.base_url, collection_id);
            let delete_body = serde_json::json!({ "ids": chunk_ids });
            self.client.post(&delete_url).json(&delete_body).send()?;
        }

        Ok(count)
    }

    pub fn count(&self) -> usize {
        let Some(collection_id) = &self.collection_id else { return 0 };
        let url = format!("{}/collections/{}/count", self.base_url, collection_id);
        self.client.get(&url).send().ok()
            .and_then(|r| r.json().ok())
            .unwrap_or(0)
    }
}

// ============================================================================
// Indexer
// ============================================================================

pub struct CodebaseIndexer {
    chroma: ChromaClient,
    embedding_client: EmbeddingClient,
    chunker: CodeChunker,
    git_commit: String,
    git_branch: String,
}

impl CodebaseIndexer {
    pub fn new(chroma_host: &str, chroma_port: &str, collection: &str, embed_url: &str, git_commit: String, git_branch: String) -> Result<Self> {
        println!("Connecting to ChromaDB at {}:{}...", chroma_host, chroma_port);
        let chroma = ChromaClient::new(chroma_host, chroma_port, collection)?;

        println!("Connecting to embedding service at {}...", embed_url);
        let embedding_client = EmbeddingClient::new(embed_url)?;
        println!("  Ready!");

        let chunker = CodeChunker::new(git_commit.clone(), git_branch.clone());

        Ok(Self { chroma, embedding_client, chunker, git_commit, git_branch })
    }

    pub fn index(&self, directory: &Path, batch_size: usize) -> Result<()> {
        println!("Indexing {}...", directory.display());

        // Check if already indexed
        if !self.git_commit.is_empty() && !self.git_branch.is_empty() {
            if self.chroma.is_commit_indexed(&self.git_branch, &self.git_commit) {
                println!("Branch {} at commit {} already indexed.", self.git_branch, &self.git_commit[..8.min(self.git_commit.len())]);
                println!("Total chunks: {}", self.chroma.count());
                return Ok(());
            }

            // Clean up old commits for this branch
            let deleted = self.chroma.delete_old_commits(&self.git_branch, &self.git_commit)?;
            if deleted > 0 {
                println!("Cleaned up {} old chunks", deleted);
            }
        }

        // Scan files
        println!("Scanning...");
        let files = self.scan_directory(directory)?;
        println!("Found {} files", files.len());

        if files.is_empty() {
            return Ok(());
        }

        // Process files in parallel to generate chunks
        let processed = Arc::new(Mutex::new(0usize));
        let total = files.len();

        let chunks: Vec<Chunk> = files
            .par_iter()
            .filter_map(|path| {
                let content = fs::read_to_string(path).ok()?;
                if content.is_empty() { return None; }

                let relative = path.strip_prefix(directory).unwrap_or(path).to_string_lossy().to_string();
                let file_chunks = self.chunker.chunk_code(&content, &relative);

                let mut count = processed.lock().unwrap();
                *count += 1;
                if *count % 100 == 0 {
                    println!("Processed {}/{} files", *count, total);
                }

                Some(file_chunks)
            })
            .flatten()
            .collect();

        println!("Generated {} chunks", chunks.len());

        // Upload with pipelining
        let total_batches = (chunks.len() + batch_size - 1) / batch_size;
        let batches: Vec<_> = chunks.chunks(batch_size).collect();

        let (tx, rx) = mpsc::channel::<(Vec<Chunk>, Vec<Vec<f32>>)>();
        let chroma = self.chroma.clone();

        let upload_thread = thread::spawn(move || -> Result<()> {
            while let Ok((chunks, embeddings)) = rx.recv() {
                chroma.add_chunks(&chunks, embeddings)?;
            }
            Ok(())
        });

        for (i, batch) in batches.iter().enumerate() {
            println!("Batch {}/{}", i + 1, total_batches);
            let texts: Vec<&str> = batch.iter().map(|c| c.text.as_str()).collect();
            let embeddings = self.embedding_client.encode(&texts)?;
            tx.send((batch.to_vec(), embeddings)).ok();
        }

        drop(tx);
        upload_thread.join().map_err(|_| anyhow::anyhow!("Upload thread panicked"))??;

        println!("Done! Total chunks: {}", self.chroma.count());
        Ok(())
    }

    fn scan_directory(&self, directory: &Path) -> Result<Vec<PathBuf>> {
        let gitignore = load_gitignore(directory);
        let ignore_dirs: HashSet<&str> = ALWAYS_IGNORE_DIRS.iter().cloned().collect();
        let mut files = Vec::new();

        for entry in walkdir::WalkDir::new(directory)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let path = e.path();
                let is_dir = e.file_type().is_dir();

                if is_dir {
                    let name = e.file_name().to_str().unwrap_or("");
                    if ignore_dirs.contains(name) {
                        return false;
                    }
                }

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

            // Skip large files (>10MB)
            if let Ok(meta) = path.metadata() {
                if meta.len() > 10 * 1024 * 1024 { continue; }
            }

            files.push(path.to_path_buf());
        }

        Ok(files)
    }
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "indexer", about = "Index codebase for vector search")]
struct Args {
    #[arg(long)]
    directory: String,
    #[arg(long, default_value = "chromadb")]
    host: String,
    #[arg(long, default_value = "8000")]
    port: String,
    #[arg(long, default_value = "codebase")]
    collection: String,
    #[arg(long, default_value_t = 128)]
    batch_size: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let directory = PathBuf::from(&args.directory);

    if !directory.is_dir() {
        anyhow::bail!("{} is not a directory", args.directory);
    }

    let git_commit = env::var("GIT_HASH").unwrap_or_default();
    let git_branch = env::var("GIT_BRANCH").unwrap_or_default();
    let embed_url = env::var("TEI_URL").unwrap_or_else(|_| "http://localhost:8081".to_string());

    println!("=== Rust Codebase Indexer ===");
    println!("Directory: {}", args.directory);
    println!("TEI: {}", embed_url);
    println!("Collection: {}", args.collection);
    if !git_branch.is_empty() { println!("Git branch: {}", git_branch); }
    if !git_commit.is_empty() { println!("Git commit: {}", &git_commit[..8.min(git_commit.len())]); }

    let indexer = CodebaseIndexer::new(&args.host, &args.port, &args.collection, &embed_url, git_commit, git_branch)?;
    indexer.index(&directory, args.batch_size)?;

    Ok(())
}
