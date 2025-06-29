#!/bin/bash

ROOT_DIR=".."

find "$ROOT_DIR" -name "Cargo.toml" -execdir sh -c '
    project_dir=$(pwd);

    cargo upgrade --incompatible --manifest-path "$project_dir/Cargo.toml";
' {} \;