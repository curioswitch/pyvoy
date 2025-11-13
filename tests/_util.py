from __future__ import annotations

import asyncio
from asyncio import StreamReader


async def find_logs_lines(
    logs: StreamReader, expected_lines: list[str]
) -> tuple[set[str], list[str]]:
    found_lines: set[str] = set()
    read_lines: list[str] = []
    try:
        async for line in logs:
            read_lines.append(line.decode())
            decoded_line = line.decode().rstrip()
            for expected_line in expected_lines:
                if expected_line in decoded_line:
                    found_lines.add(expected_line)
            if len(found_lines) == len(expected_lines):
                break
    except asyncio.CancelledError:
        pass
    return found_lines, read_lines


async def assert_logs_contains(
    logs: StreamReader, expected_lines: list[str]
) -> list[str]:
    found_lines, read_lines = await asyncio.wait_for(
        find_logs_lines(logs, expected_lines), timeout=3.0
    )
    missing_lines = set(expected_lines) - found_lines
    assert not missing_lines, (  # noqa: S101
        f"Missing log lines: {missing_lines}\nRead lines: {''.join(read_lines)}"
    )
    return read_lines
