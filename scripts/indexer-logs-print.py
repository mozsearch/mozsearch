#!/usr/bin/env python3

# Consumes the NDJSON fed to us by the indexer-logs-analyze.sh script which is
# an exciting bunch of shell data that eventually ends up as JSON that it's just
# most practical to actual be using a high level language on at this point.

import datetime
import json
import sys

from rich import box, print
from rich.console import Console
from rich.markup import escape
from rich.table import Table
from rich.tree import Tree
from rich.traceback import install

class IndexerLogStdinPrinter:
    def __init__(self):
        self.data_by_tree = {}

    def consume(self, fp):
        for line in fp.readlines():
            data = json.loads(line)
            if 'tree' not in data:
                continue

            tree = data['tree']
            dtime = data['time'] = datetime.datetime.fromisoformat(data['time'])

            if tree not in self.data_by_tree:
                tree_data = {
                    'tree': tree,
                    'data': [],
                    'first': dtime,
                    'last': dtime,
                }
                self.data_by_tree[tree] = tree_data
            else:
                tree_data = self.data_by_tree[tree]
            data['dur'] = ''
            if len(tree_data['data']):
                prev_data = tree_data['data'][-1]
                prev_data['dur'] = data['time'] - prev_data['time']
            tree_data['data'].append(data)
            if data['time'] > tree_data['last']:
                tree_data['last'] = data['time']

    def print(self):
        root_tree = Tree('Trees')
        for tree_name, tree_data in self.data_by_tree.items():
            name_with_dur = f"{tree_name} - {tree_data['last'] - tree_data['first']}"
            tree_tree = root_tree.add(tree_name)

            table = Table(box=box.SIMPLE)
            table.add_column('script')
            table.add_column('time since start')
            table.add_column('apparent duration')

            for data in tree_data['data']:
                table.add_row(
                    data['script'],
                    f"{data['time'] - tree_data['first']}",
                    f"{data['dur']}",
                )

            tree_tree.add(table)
        print(root_tree)

if __name__ == '__main__':
    install(show_locals=True)
    cmd = IndexerLogStdinPrinter()
    cmd.consume(sys.stdin)
    cmd.print()
