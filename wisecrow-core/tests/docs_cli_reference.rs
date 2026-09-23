//! The CLI reference must name every command the binary accepts.
//!
//! A command that exists and is undocumented is one nobody can find. The list
//! is taken from clap rather than from a hand-kept table here, so adding a
//! command fails this test until the reference mentions it.

use clap::CommandFactory;
use wisecrow::cli::Cli;

const REFERENCE: &str = include_str!("../../docs/src/reference/cli-reference.md");

#[test]
fn every_command_is_named_in_the_cli_reference() {
    let command = Cli::command();
    let missing: Vec<&str> = command
        .get_subcommands()
        .map(clap::Command::get_name)
        .filter(|name| !REFERENCE.contains(&format!("`{name}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "undocumented commands: {missing:?}; add them to docs/src/reference/cli-reference.md"
    );
}

#[test]
fn every_command_alias_is_named_in_the_cli_reference() {
    let mut missing: Vec<String> = Vec::new();
    let root = Cli::command();
    for command in root.get_subcommands() {
        for alias in command.get_all_aliases() {
            if !REFERENCE.contains(&format!("`{alias}`")) {
                missing.push(format!("{} ({alias})", command.get_name()));
            }
        }
    }
    assert!(missing.is_empty(), "undocumented aliases: {missing:?}");
}
