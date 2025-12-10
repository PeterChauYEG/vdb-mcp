#!/usr/bin/env python3
"""
Codebase Indexer for Vector MCP Server

Indexes a codebase into ChromaDB for semantic search with branch-based indexing.
"""

import argparse
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import List, Dict, Optional
import threading

import chromadb
from chromadb.config import Settings
from sentence_transformers import SentenceTransformer

# Import local modules
from indexer.file_utils import (
    load_gitignore,
    extract_directory_ignores,
    should_index_file,
    get_file_mtime,
    hash_file,
    ALWAYS_IGNORE_DIRS,
)
from indexer.branch_manager import BranchManager
from indexer.chunker import CodeChunker


class CodebaseIndexer:
    """Main indexer class that orchestrates the indexing process."""

    def __init__(
        self,
        chroma_host: str = "localhost",
        chroma_port: int = 8000,
        collection_name: str = "codebase",
        embedding_model: str = "all-MiniLM-L6-v2",
        git_hash: str = "",
        git_branch: str = "",
    ):
        """Initialize the codebase indexer."""
        self.chroma_client = chromadb.HttpClient(
            host=chroma_host,
            port=chroma_port,
            settings=Settings(anonymized_telemetry=False),
        )
        self.collection_name = collection_name
        self.embedding_model_name = embedding_model
        self.git_hash = git_hash
        self.git_branch = git_branch
        self.lock = threading.Lock()

        # Initialize embedding model (CPU only)
        print("Using CPU for embeddings")
        print(f"Loading embedding model: {embedding_model}")
        self.embedding_model = SentenceTransformer(embedding_model, device="cpu")
        print("Embedding model loaded successfully")

        # Get or create collection
        self.collection = self._get_or_create_collection()

        # Initialize helper classes
        self.branch_manager = BranchManager(self.collection, git_branch, git_hash)
        self.chunker = CodeChunker(git_branch, git_hash)

    def _get_or_create_collection(self):
        """Get existing collection or create a new one."""
        collection_metadata = {
            "description": "Codebase semantic search",
            "git_hash": self.git_hash,
            "indexed_at": str(int(time.time())),
        }

        try:
            collection = self.chroma_client.get_collection(name=self.collection_name)
            print(f"Using existing collection: {self.collection_name}")
            # Update git hash
            if self.git_hash:
                print(f"Updating git hash: {self.git_hash[:8]}")
        except Exception:
            collection = self.chroma_client.create_collection(
                name=self.collection_name, metadata=collection_metadata
            )
            print(f"Created new collection: {self.collection_name}")

        return collection

    def _process_single_file(
        self,
        file_path: Path,
        directory: Path,
        max_file_size_mb: int = 10,
    ) -> List[tuple[str, Dict]]:
        """
        Process a single file and return its chunks.

        Args:
            file_path: Path to the file
            directory: Base directory (for relative paths)
            max_file_size_mb: Maximum file size in MB

        Returns:
            List of (chunk_text, metadata) tuples
        """
        try:
            file_size_mb = file_path.stat().st_size / (1024 * 1024)
            if file_size_mb > max_file_size_mb:
                print(f"  Skipping {file_path.name} (too large: {file_size_mb:.1f}MB)")
                return []

            with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
                content = f.read()

            relative_path = str(file_path.relative_to(directory))
            file_hash = hash_file(file_path)

            return self.chunker.chunk_code(content, relative_path, file_hash)

        except Exception as e:
            print(f"Error processing {file_path}: {e}")
            return []

    def index_directory(
        self,
        directory: str,
        batch_size: int = 500,
        incremental: bool = True,
        max_workers: int = 8,
        max_file_size_mb: int = 10,
    ):
        """
        Index all code files in a directory.

        Args:
            directory: Path to the directory to index
            batch_size: Number of chunks to batch before adding to collection
            incremental: Only index changed files
            max_workers: Maximum number of parallel workers
            max_file_size_mb: Maximum file size in MB
        """
        directory = Path(directory).resolve()
        print(f"\nIndexing directory: {directory}")

        # Check if branch+commit already indexed
        if self.git_branch and incremental:
            if self.branch_manager.check_branch_indexed():
                print(
                    f"✅ Branch '{self.git_branch}' already indexed at commit {self.git_hash[:8]}"
                )
                print("No indexing needed!")
                return

            # Clean up old commits for this branch
            self.branch_manager.cleanup_old_branch()
            print()

        # Load .gitignore and extract directory patterns
        gitignore_spec = load_gitignore(directory)
        gitignore_dirs = extract_directory_ignores(directory)
        if gitignore_spec:
            print("Loaded .gitignore patterns")
            if gitignore_dirs:
                print(f"Extracted {len(gitignore_dirs)} directory ignore patterns")

        # Get indexed files if doing incremental update
        indexed_files = {}
        if incremental:
            print("Checking for previously indexed files...")
            indexed_files = self.branch_manager.get_indexed_files_with_mtime()
            print(f"Found {len(indexed_files)} previously indexed files")

        # Scan directory for files
        all_files, files_to_index = self._scan_directory(
            directory, gitignore_spec, gitignore_dirs, indexed_files, incremental
        )

        print(f"Found {len(all_files)} total files")
        if incremental and len(all_files) - len(files_to_index) > 0:
            print(f"Skipping {len(all_files) - len(files_to_index)} unchanged files")
        print(f"Processing {len(files_to_index)} files:")

        # Count new vs modified
        new_count = sum(
            1
            for f in files_to_index
            if str(f.relative_to(directory)) not in indexed_files
        )
        modified_count = len(files_to_index) - new_count
        if new_count > 0:
            print(f"  - {new_count} new files")
        if modified_count > 0:
            print(f"  - {modified_count} modified files")

        if not files_to_index:
            print("\n✅ Indexing complete! No files needed indexing.")
            self.get_stats()
            return

        # Delete old chunks for modified files (batch operation)
        if incremental and modified_count > 0:
            print(f"Deleting old chunks for {modified_count} modified files...")
            for file_path in files_to_index:
                relative_path = str(file_path.relative_to(directory))
                if relative_path in indexed_files:
                    self.branch_manager.delete_file_chunks(relative_path)
            print("Deletion complete")

        # Process files in parallel
        print(f"Using {min(max_workers, len(files_to_index))} parallel workers\n")
        self._process_files_parallel(
            files_to_index, directory, max_workers, max_file_size_mb, batch_size
        )

        print(f"\n✅ Indexing complete! Total documents in collection: {self.collection.count()}")
        self.get_stats()

    def _scan_directory(
        self,
        directory: Path,
        gitignore_spec,
        gitignore_dirs: set[str],
        indexed_files: Dict,
        incremental: bool,
    ) -> tuple[List[Path], List[Path]]:
        """Scan directory and determine which files need indexing."""
        all_files = []
        files_to_index = []
        gitignored_count = 0
        dirs_scanned = 0
        last_progress_time = time.time()

        # Combine hardcoded ignores with gitignore directory patterns
        ignore_dirs = ALWAYS_IGNORE_DIRS | gitignore_dirs

        print("Scanning codebase...")
        for root, dirs, files in os.walk(directory, followlinks=False):
            dirs_scanned += 1

            # Show progress
            current_time = time.time()
            if dirs_scanned % 100 == 0 or (current_time - last_progress_time) > 2:
                print(
                    f"  Scanned {dirs_scanned} directories, found {len(all_files)} files so far..."
                )
                last_progress_time = current_time

            root_path = Path(root)

            # Filter directories using combined ignore set
            dirs[:] = [d for d in dirs if d not in ignore_dirs]

            # Additional filtering based on full gitignore patterns (for paths)
            if gitignore_spec:
                filtered_dirs = []
                for d in dirs:
                    dir_path = root_path / d
                    relative_dir = str(dir_path.relative_to(directory))
                    if not gitignore_spec.match_file(
                        relative_dir
                    ) and not gitignore_spec.match_file(relative_dir + "/"):
                        filtered_dirs.append(d)
                    else:
                        gitignored_count += 1
                dirs[:] = filtered_dirs

            # Process files
            for file in files:
                file_path = Path(root) / file

                if not file_path.exists() or not file_path.is_file():
                    continue

                # Check gitignore
                if gitignore_spec:
                    relative_path = str(file_path.relative_to(directory))
                    if gitignore_spec.match_file(relative_path):
                        gitignored_count += 1
                        continue

                if should_index_file(file_path):
                    all_files.append(file_path)

                    # Check if needs indexing
                    if incremental:
                        relative_path = str(file_path.relative_to(directory))
                        if self._needs_reindex(file_path, relative_path, indexed_files):
                            files_to_index.append(file_path)
                    else:
                        files_to_index.append(file_path)

        if gitignore_spec and gitignored_count > 0:
            print(f"Ignored {gitignored_count} files/directories from .gitignore")

        return all_files, files_to_index

    def _needs_reindex(
        self, file_path: Path, relative_path: str, indexed_files: Dict
    ) -> bool:
        """Determine if a file needs re-indexing."""
        # New file - needs indexing
        if relative_path not in indexed_files:
            return True

        # For branch-based indexing, only reindex if git commit changed
        indexed_info = indexed_files[relative_path]
        indexed_git_hash = indexed_info.get("git_hash", "")
        if self.git_hash and indexed_git_hash and self.git_hash != indexed_git_hash:
            return True

        # File hasn't changed
        return False

    def _process_files_parallel(
        self,
        files_to_index: List[Path],
        directory: Path,
        max_workers: int,
        max_file_size_mb: int,
        batch_size: int,
    ):
        """Process files in parallel and add to collection in batches."""
        all_chunks = []
        all_metadatas = []
        all_ids = []

        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            futures = {
                executor.submit(
                    self._process_single_file, file_path, directory, max_file_size_mb
                ): file_path
                for file_path in files_to_index
            }

            for future in as_completed(futures):
                chunks = future.result()
                for chunk_text, metadata in chunks:
                    all_chunks.append(chunk_text)
                    all_metadatas.append(metadata)
                    # Include branch, commit, and line range to ensure unique IDs
                    chunk_id = f"{metadata['git_branch']}_{metadata['git_commit'][:8]}_{metadata['file_path']}_{metadata['start_line']}_{metadata['end_line']}"
                    all_ids.append(chunk_id)

                    if len(all_chunks) >= batch_size:
                        self._add_batch(all_chunks, all_metadatas, all_ids)
                        all_chunks = []
                        all_metadatas = []
                        all_ids = []

        # Add remaining chunks
        if all_chunks:
            self._add_batch(all_chunks, all_metadatas, all_ids)

    def _add_batch(self, chunks: List[str], metadatas: List[Dict], ids: List[str]):
        """Add a batch of chunks to the collection."""
        with self.lock:
            embeddings = self.embedding_model.encode(
                chunks, convert_to_numpy=True
            ).tolist()
            self.collection.add(
                documents=chunks, metadatas=metadatas, ids=ids, embeddings=embeddings
            )

    def get_stats(self) -> Dict:
        """Get collection statistics."""
        stats = {
            "name": self.collection_name,
            "count": self.collection.count(),
            "model": self.embedding_model_name,
        }
        print("\n📊 Collection Stats:")
        print(f"   Name: {stats['name']}")
        print(f"   Documents: {stats['count']}")
        print(f"   Model: {stats['model']}")
        return stats


def main():
    parser = argparse.ArgumentParser(
        description="Index a codebase into ChromaDB for semantic search"
    )
    parser.add_argument(
        "directory", help="Directory to index (e.g., /path/to/your/codebase)"
    )
    parser.add_argument(
        "--host", default="localhost", help="ChromaDB host (default: localhost)"
    )
    parser.add_argument(
        "--port", type=int, default=8000, help="ChromaDB port (default: 8000)"
    )
    parser.add_argument(
        "--collection", default="codebase", help="Collection name (default: codebase)"
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=500,
        help="Batch size for processing (default: 500)",
    )
    parser.add_argument(
        "--model",
        default="all-MiniLM-L6-v2",
        help="SentenceTransformer model (default: all-MiniLM-L6-v2)",
    )
    parser.add_argument(
        "--max-workers",
        type=int,
        default=8,
        help="Maximum number of parallel workers (default: 8)",
    )
    parser.add_argument(
        "--no-incremental",
        action="store_true",
        help="Disable incremental indexing (re-index all files)",
    )
    parser.add_argument(
        "--max-file-size",
        type=int,
        default=10,
        help="Maximum file size to index in MB (default: 10)",
    )

    args = parser.parse_args()

    if not os.path.isdir(args.directory):
        print(f"Error: Directory not found: {args.directory}")
        sys.exit(1)

    try:
        # Get git hash and branch from environment
        git_hash = os.environ.get("GIT_HASH", "")
        git_branch = os.environ.get("GIT_BRANCH", "")

        indexer = CodebaseIndexer(
            chroma_host=args.host,
            chroma_port=args.port,
            collection_name=args.collection,
            embedding_model=args.model,
            git_hash=git_hash,
            git_branch=git_branch,
        )

        indexer.index_directory(
            directory=args.directory,
            batch_size=args.batch_size,
            incremental=not args.no_incremental,
            max_workers=args.max_workers,
            max_file_size_mb=args.max_file_size,
        )

    except KeyboardInterrupt:
        print("\n\nIndexing interrupted by user")
        sys.exit(0)
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback

        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
