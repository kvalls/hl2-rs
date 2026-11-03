#!/usr/bin/env python3
"""Each application plugin selection builds and pulls only the selected plugins into its tree."""
import subprocess
import sys

PLUGINS = {"advanced-options": "hl2-advanced-options", "debug-tools": "hl2-debug"}
SELECTIONS = [[], ["advanced-options"], ["debug-tools"], ["advanced-options", "debug-tools"]]


def cargo(command, selection):
    arguments = ["cargo", command, "--locked", "-p", "hl2-app", "--no-default-features"]
    if selection:
        arguments += ["--features", ",".join(selection)]
    if command == "tree":
        arguments += ["-e", "normal", "--prefix", "none"]
    return subprocess.run(arguments, check=True, capture_output=True, text=True).stdout


def unexpected_packages(tree, selection):
    """Workspace plugins in a `cargo tree` listing that `selection` did not ask for."""
    packages = {line.split()[0] for line in tree.splitlines() if line.strip()}
    return sorted(crate for feature, crate in PLUGINS.items()
                  if feature not in selection and crate in packages)


def main():
    failed = False
    for selection in SELECTIONS:
        name = "+".join(selection) or "core only"
        leaked = unexpected_packages(cargo("tree", selection), selection)
        if leaked:
            failed = True
            print(f"FAIL {name}: the dependency tree contains {', '.join(leaked)}")
        cargo("check", selection)
        print(f"ok {name}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
