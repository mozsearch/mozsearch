#!/usr/bin/env python3

import json
import sys

config_input_path = sys.argv[1]
config_output_path = sys.argv[2]
mozsearch_path = sys.argv[3]
# NOTE: This is not always the mozsearch-mozilla repository.
config_repo = sys.argv[4]
working = sys.argv[5]
mozsearch_source_path = sys.argv[6]

def load_config(path):
    with open(path) as f:
        return json.loads(
            f.read()
            .replace('$MOZSEARCH_PATH', mozsearch_path)
            .replace('$CONFIG_REPO', config_repo)
            .replace('$WORKING', working)
            .replace('$MOZSEARCH_SOURCE_PATH', mozsearch_source_path)
        )

config = load_config(config_input_path)

for tree in config["trees"].values():
    if "IMPORT" in tree:
        source = tree["IMPORT"]
        sub_config = load_config(source["file"])
        tree_name = source["tree_name"]
        for k, v in sub_config["trees"][tree_name].items():
            if k not in tree:
                tree[k] = v
        del tree["IMPORT"]

with open(config_output_path, 'w') as f:
    json.dump(config, f, indent=2)
