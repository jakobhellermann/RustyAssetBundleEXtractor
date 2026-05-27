update-tpk-embed:
    curl -L -o resources/lz4.tpk.zip https://nightly.link/AssetRipper/Tpk/workflows/type_tree_tpk/master/lz4_file.zip
    unzip -o -j resources/lz4.tpk.zip -d resources/
    rm resources/lz4.tpk.zip

update-class-id:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=utils/class_generator/TypeTreeDumps
    if [ -d "$dir/.git" ]; then
        git -C "$dir" fetch --depth=1 origin main
        git -C "$dir" reset --hard origin/main
    else
        git clone --depth=1 --filter=blob:none --sparse https://github.com/AssetRipper/TypeTreeDumps "$dir"
        git -C "$dir" sparse-checkout set Classes
    fi
    python3 utils/class_generator/generate_class_id.py
    cargo fmt -- src/objects/class_id.rs
