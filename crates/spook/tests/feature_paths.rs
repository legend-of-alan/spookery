//! Feature folders, each owned by one cucumber program.
//! Each program reads one list. The census reads both.

#[allow(dead_code)]
pub const HOST: &[&str] = &[
    "tests/features/cli",
    "tests/features/digest",
    "tests/features/host",
    "tests/features/limits",
    "tests/features/loopback",
    "tests/features/mcp",
    "tests/features/new",
    "tests/features/seats",
    "tests/features/speech",
];

#[allow(dead_code)]
pub const WORLD: &[&str] = &[
    "tests/features/boundary",
    "tests/features/catchup",
    "tests/features/check",
    "tests/features/checks",
    "tests/features/clocks",
    "tests/features/commit",
    "tests/features/firewall",
    "tests/features/idle",
    "tests/features/items",
    "tests/features/map",
    "tests/features/perceive",
    "tests/features/privacy",
    "tests/features/readme",
    "tests/features/replay",
    "tests/features/round",
    "tests/features/rounds",
    "tests/features/rows",
    "tests/features/rules",
    "tests/features/sample",
    "tests/features/stock",
    "tests/features/wake",
];
