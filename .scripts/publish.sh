#!/bin/bash

cd ..

rm -r publish
mkdir ./publish
mkdir ./publish/dist


# FRONTEND

cd ui
npm install --force
npm run publish

cp -r dist ../publish

# BACKEND

cd ..
cargo build --release
cp target/release/open_football.exe publish/open_football.exe