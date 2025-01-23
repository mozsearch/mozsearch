{
  writeShellApplication,
  mozsearch-scripts,
}:
writeShellApplication {
  name = "serve-index";

  text = ''
    export LC_ALL=C.UTF-8

    INDEX_DIR=$1
    SERVER_DIR=$2

    mkdir -p "$SERVER_DIR"
    ${mozsearch-scripts}/infrastructure/web-server-setup.sh "$INDEX_DIR" "$INDEX_DIR/config.json" "$INDEX_DIR" "$SERVER_DIR"
    ${mozsearch-scripts}/infrastructure/web-server-run.sh "$INDEX_DIR" "$INDEX_DIR" "$SERVER_DIR" WAIT
  '';
}
