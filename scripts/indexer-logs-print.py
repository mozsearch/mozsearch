#!/usr/bin/env python3

import datetime
import re
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
            m = re.match('Performing (.+) step for (.+) : (.+)', line)
            if m is None:
                continue

            dtime = datetime.datetime.fromisoformat(m.group(3))

            tree = m.group(2)

            data = {
                'script': m.group(1),
                'time': dtime,
                'dur': '',
            }

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
