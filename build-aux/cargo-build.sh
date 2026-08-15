#!/bin/sh
set -eu

source_directory=$1
output_file=$2
target_directory=$3
locale_directory=$4

case "$output_file" in
  /*) ;;
  *) output_file="$PWD/$output_file" ;;
esac

cd "$source_directory"
LOCALEDIR="$locale_directory" CARGO_TARGET_DIR="$target_directory" \
  cargo build --release --locked
cp "$target_directory/release/remind-me" "$output_file"
