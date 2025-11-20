# Includes code from:

# Copyright © 2017-present, [Encode OSS Ltd](https://www.encode.io/).
# All rights reserved.
# SPDX-License-Identifier: BSD-3-Clause
from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

try:
    import watchfiles

    WATCH_SUPPORTED = True
except ImportError:
    WATCH_SUPPORTED = False


class FileFilter:
    def __init__(self, includes: list[str], excludes: list[str]) -> None:
        default_includes = ["*.py"]
        default_excludes = [".*", ".py[cod]", ".sw.*", "~*"]

        self._includes = [
            default for default in default_includes if default not in excludes
        ]
        self._includes.extend(includes)
        self._includes = list(set(self._includes))

        self._excludes = [
            default for default in default_excludes if default not in includes
        ]
        self._exclude_dirs = []
        for e in excludes:
            p = Path(e)
            try:
                is_dir = p.is_dir()
            except OSError:
                # gets raised on Windows for values like "*.py"
                is_dir = False

            if is_dir:
                self._exclude_dirs.append(p)
            else:
                self._excludes.append(e)
        self._excludes = list(set(self._excludes))

    def __call__(self, _change: watchfiles.Change, p: str) -> bool:
        path = Path(p)
        for include_pattern in self._includes:
            if path.match(include_pattern):
                if str(path).endswith(include_pattern):
                    return True

                for exclude_dir in self._exclude_dirs:
                    if exclude_dir in path.parents:
                        return False

                for exclude_pattern in self._excludes:
                    if path.match(exclude_pattern):
                        return False

                return True
        return False


async def watch(
    dirs: list[Path], includes: list[str], excludes: list[str]
) -> AsyncIterator[list[str]]:
    if not WATCH_SUPPORTED:
        msg = "watchfiles not found, add it to dev dependencies to enable reload functionality"
        raise RuntimeError(msg)

    async for changes in watchfiles.awatch(
        *dirs, watch_filter=FileFilter(includes, excludes)
    ):
        if changes:
            unique_paths = {path for _change, path in changes}
            yield [_display_path(Path(p)) for p in unique_paths]


def _display_path(path: Path) -> str:
    try:
        return f"'{path.relative_to(Path.cwd())}'"
    except ValueError:
        return f"'{path}'"
