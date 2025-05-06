#!/bin/bash

ROOT_DIR=".."

find "$ROOT_DIR" -name "Cargo.toml" -execdir sh -c '
    project_dir=$(pwd);

    cargo update --manifest-path "$project_dir/Cargo.toml";
    cargo upgrade -i --manifest-path "$project_dir/Cargo.toml";
' {} \;