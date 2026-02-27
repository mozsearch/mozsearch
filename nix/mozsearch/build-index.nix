{
  writeShellApplication,
  mozsearch-scripts,
}:
writeShellApplication {
  name = "build-index";

  text = ''
    export LC_ALL=C.UTF-8

    CONFIG_DIR=$1
    CONFIG=$2
    INDEX_DIR=$3

    mkdir -p "$INDEX_DIR"
    ${mozsearch-scripts}/infrastructure/indexer-setup.sh "$CONFIG_DIR" "$CONFIG" "$INDEX_DIR"
    ${mozsearch-scripts}/infrastructure/indexer-run.sh "$CONFIG_DIR" "$INDEX_DIR"
  '';
}
