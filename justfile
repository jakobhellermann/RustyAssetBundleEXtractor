update-tpk-embed:
    curl -L -o resources/lz4.tpk.zip https://nightly.link/AssetRipper/Tpk/workflows/type_tree_tpk/master/lz4_file.zip
    unzip -o -j resources/lz4.tpk.zip -d resources/
    rm resources/lz4.tpk.zip
