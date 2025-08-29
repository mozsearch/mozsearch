{
  writeShellApplication,
  mozsearch-scripts,
}:
writeShellApplication {
  name = "serve-index";

  text = ''
    export LC_ALL=C.UTF-8

    CONFIG_DIR=$1
    CONFIG=$2
    INDEX_DIR=$3
    SERVER_DIR=$4

    mkdir -p "$SERVER_DIR"
    ${mozsearch-scripts}/infrastructure/web-server-setup.sh "$CONFIG_DIR" "$CONFIG" "$INDEX_DIR" "$SERVER_DIR"
    ${mozsearch-scripts}/infrastructure/web-server-run.sh "$CONFIG_DIR" "$INDEX_DIR" "$SERVER_DIR" WAIT
  '';
}
