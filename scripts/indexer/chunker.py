"""Code chunking utilities for the codebase indexer."""

import time
from pathlib import Path
from typing import List, Tuple, Dict

from .file_utils import hash_content


class CodeChunker:
    """Handles chunking of code files into manageable pieces."""

    def __init__(self, git_branch: str, git_hash: str):
        """
        Initialize chunker.

        Args:
            git_branch: Current git branch name
            git_hash: Current git commit hash
        """
        self.git_branch = git_branch
        self.git_hash = git_hash

    def chunk_code(
        self,
        content: str,
        file_path: str,
        file_hash: str = "",
        chunk_size: int = 2000,
        overlap: int = 400,
    ) -> List[Tuple[str, Dict]]:
        """
        Split code into overlapping chunks with metadata.

        Args:
            content: File content
            file_path: Relative path to the file
            file_hash: SHA-256 hash of entire file
            chunk_size: Target chunk size in characters
            overlap: Overlap between chunks in characters

        Returns:
            List of (chunk_text, metadata) tuples
        """
        lines = content.split("\n")
        chunks = []
        current_chunk = []
        current_size = 0
        start_line = 1

        for i, line in enumerate(lines, start=1):
            line_size = len(line) + 1  # +1 for newline

            if current_size + line_size > chunk_size and current_chunk:
                # Save current chunk
                chunk_text = "\n".join(current_chunk)
                metadata = self._create_metadata(
                    file_path, start_line, i - 1, chunk_text, file_hash
                )
                chunks.append((chunk_text, metadata))

                # Start new chunk with overlap
                overlap_lines = self._get_overlap_lines(current_chunk, overlap)
                current_chunk = overlap_lines + [line]
                current_size = sum(len(l) + 1 for l in overlap_lines) + line_size
                start_line = i - len(overlap_lines)
            else:
                current_chunk.append(line)
                current_size += line_size

        # Add final chunk
        if current_chunk:
            chunk_text = "\n".join(current_chunk)
            metadata = self._create_metadata(
                file_path, start_line, len(lines), chunk_text, file_hash
            )
            chunks.append((chunk_text, metadata))

        return chunks

    def _create_metadata(
        self,
        file_path: str,
        start_line: int,
        end_line: int,
        chunk_text: str,
        file_hash: str,
    ) -> Dict:
        """Create metadata dictionary for a chunk."""
        return {
            "file_path": file_path,
            "start_line": start_line,
            "end_line": end_line,
            "file_type": Path(file_path).suffix,
            "content_hash": hash_content(chunk_text),
            "file_hash": file_hash,
            "git_commit": self.git_hash,
            "git_branch": self.git_branch,
            "indexed_at": int(time.time()),
        }

    def _get_overlap_lines(self, current_chunk: List[str], overlap: int) -> List[str]:
        """Extract overlap lines from end of current chunk."""
        overlap_lines = []
        overlap_size = 0

        for prev_line in reversed(current_chunk):
            line_size = len(prev_line) + 1
            if overlap_size + line_size <= overlap:
                overlap_lines.insert(0, prev_line)
                overlap_size += line_size
            else:
                break

        return overlap_lines
