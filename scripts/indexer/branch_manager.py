"""Branch management for the codebase indexer."""

from typing import Optional


class BranchManager:
    """Manages git branch operations for the indexer."""

    def __init__(self, collection, git_branch: str, git_hash: str):
        """
        Initialize branch manager.

        Args:
            collection: ChromaDB collection
            git_branch: Current git branch name
            git_hash: Current git commit hash
        """
        self.collection = collection
        self.git_branch = git_branch
        self.git_hash = git_hash

    def check_branch_indexed(self) -> bool:
        """Check if current branch+commit combo is already indexed."""
        if not self.git_branch or not self.git_hash:
            return False

        try:
            results = self.collection.get(
                where={"git_branch": self.git_branch, "git_commit": self.git_hash},
                limit=1,
            )
            return len(results.get("ids", [])) > 0
        except Exception:
            return False

    def cleanup_old_branch(self):
        """Delete old chunks for current branch (keep only latest commit)."""
        if not self.git_branch:
            return

        try:
            print(f"Cleaning up old chunks for branch: {self.git_branch}")
            results = self.collection.get(
                where={"git_branch": self.git_branch}, include=["metadatas"]
            )

            ids_to_delete = []
            for idx, metadata in enumerate(results.get("metadatas", [])):
                if metadata.get("git_commit") != self.git_hash:
                    ids_to_delete.append(results["ids"][idx])

            if ids_to_delete:
                print(f"  Removing {len(ids_to_delete)} old chunks")
                self.collection.delete(ids=ids_to_delete)
        except Exception as e:
            print(f"Warning: Could not cleanup old branch chunks: {e}")

    def get_indexed_files_with_mtime(self) -> dict[str, dict]:
        """
        Get all indexed files with their metadata.

        Returns:
            Dictionary mapping relative file paths to their metadata
        """
        try:
            results = self.collection.get(include=["metadatas"])

            files = {}
            for metadata in results.get("metadatas", []):
                file_path = metadata.get("file_path", "")
                if file_path and file_path not in files:
                    files[file_path] = {
                        "mtime": metadata.get("mtime", 0),
                        "git_hash": metadata.get("git_commit", ""),
                        "file_hash": metadata.get("file_hash", ""),
                    }
            return files
        except Exception as e:
            print(f"Warning: Could not get indexed files: {e}")
            return {}

    def delete_file_chunks(self, relative_path: str):
        """Delete all chunks for a specific file."""
        try:
            self.collection.delete(where={"file_path": relative_path})
        except Exception as e:
            print(f"Warning: Could not delete chunks for {relative_path}: {e}")
