"""File utilities for the codebase indexer."""

import hashlib
from pathlib import Path
from typing import Optional

import pathspec


# Minimal hardcoded ignores (only things that should NEVER be indexed)
# Note: .yarn has negation patterns in gitignore but should always be ignored
ALWAYS_IGNORE_DIRS = {
    ".git", ".yarn", "assets", "docs", "cypress", "storybook", "__mocks__",
    ".maestro", ".github", "examples", "codemods", "msw", "fastlane",
    "code-signing", ".reassure", ".vscode", ".claude",
    # Build and generated directories
    "build", "Pods", ".gradle", "node_modules", "dist", "coverage",
    ".next", ".cache", "tmp", "temp",
    # Test and localization directories
    "test-utils", "__fixture__", "Locales", "translations"
}
ALWAYS_IGNORE_FILES = {".DS_Store"}

# Binary file extensions to skip
BINARY_EXTENSIONS = {
    # Images
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".svg", ".webp",
    # Archives
    ".zip", ".tar", ".gz", ".bz2", ".7z", ".rar",
    # Executables/Libraries
    ".exe", ".dll", ".so", ".dylib", ".a", ".o",
    # Fonts
    ".ttf", ".otf", ".woff", ".woff2", ".eot",
    # Media
    ".mp3", ".mp4", ".avi", ".mov", ".wav", ".flac",
    # Other binary formats
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
    ".db", ".sqlite", ".pyc", ".class", ".jar", ".war",  
    # models 
    ".onnx", 
    # godot
    ".pck", ".tscn"
}


def extract_directory_ignores(directory: Path) -> set[str]:
    """
    Extract directory patterns from .gitignore to use as quick filters.

    Returns a set of directory names (not paths) that should be ignored.
    This is used to quickly filter directories during os.walk before
    using the full pathspec matcher.
    """
    gitignore_path = directory / ".gitignore"
    if not gitignore_path.exists():
        return set()

    dir_ignores = set()
    with open(gitignore_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()

            # Skip comments and empty lines
            if not line or line.startswith("#"):
                continue

            # Skip negation patterns (starting with !)
            if line.startswith("!"):
                continue

            # Extract directory patterns (ending with / or containing no path separators)
            if line.endswith("/"):
                # Pattern like "node_modules/"
                dir_name = line.rstrip("/")
                if "/" not in dir_name:
                    dir_ignores.add(dir_name)
            elif "/" not in line and "*" not in line:
                # Simple pattern like "node_modules" (no wildcards, no paths)
                dir_ignores.add(line)

    return dir_ignores


def load_gitignore(directory: Path) -> Optional[pathspec.PathSpec]:
    """Load .gitignore file and return a PathSpec matcher."""
    gitignore_path = directory / ".gitignore"
    if gitignore_path.exists():
        with open(gitignore_path, "r", encoding="utf-8") as f:
            patterns = f.read().splitlines()
        return pathspec.PathSpec.from_lines("gitwildmatch", patterns)
    return None


def should_index_file(file_path: Path) -> bool:
    """Check if a file should be indexed (skip binary files and special files)."""
    if file_path.name in ALWAYS_IGNORE_FILES:
        return False

    # Skip binary files
    if file_path.suffix.lower() in BINARY_EXTENSIONS:
        return False

    # Skip test files (flexible pattern matching)
    filename = file_path.name.lower()
    path_str = str(file_path).lower()

    # Skip files with test patterns in filename
    if ".test." in filename or ".spec." in filename:
        return False

    # Skip files in __tests__ directories (anywhere in path)
    if "/__tests__/" in path_str or path_str.startswith("__tests__/"):
        return False

    # Skip files without extensions (except specific ones like Makefile, Dockerfile, etc.)
    if not file_path.suffix:
        basename = file_path.name.lower()
        allowed_no_ext = {"makefile", "dockerfile", "gemfile", "rakefile", "podfile"}
        if basename not in allowed_no_ext:
            return False

    return True


def get_file_mtime(file_path: Path) -> float:
    """Get file modification time."""
    return file_path.stat().st_mtime


def hash_content(content: str) -> str:
    """Generate SHA-256 hash of content."""
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def hash_file(file_path: Path) -> str:
    """Generate SHA-256 hash of entire file content."""
    try:
        with open(file_path, "r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
        return hash_content(content)
    except Exception:
        return ""
