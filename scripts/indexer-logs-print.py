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

# This script parses the step logs emitted while running indexer, and prints
# the statistics for each tree.
#
# The log consists of 3 tiers, section, step, and substep.
#
# A valid log is one of the following, starting from the column 1:
#
#   Performing <SECTION_NAME> section for <TREE_NAME> : <TIMESTAMP>
#   Performed <SECTION_NAME> section for <TREE_NAME> : <TIMESTAMP>
#   Performing <STEP_NAME> step for <TREE_NAME> : <TIMESTAMP>
#   Performing <STEP_NAME>::<SUBSTEP_NAME> step for <TREE_NAME> : <TIMESTAMP>
#
# TIMESTAMP uses the "YYYY-MM-DDThh:mm:ss+TZ" format.
# for example "2026-02-21T23:34:49+0900"
#
# Sections are the top-level item in the log, that corresponds to the script
# inside infrastructure directory.
# Currently, SECTION_NAME is one of the following:
#   * indexer-setup
#   * indexer-run
#   * upload
#
# "Performing" log for sections should be emitted at the beginning of the
# section, and the "Performed" log should be emitted at the end of the section.
#
# Steps are the items inside sections.
#
# If a step consists of multiple long steps, they can be emitted as
# substeps.
#
# "Performing" log for steps and substeps should be emitted at the
# beginning of each step or substep.
# The next "Performing" log or the "Performed" log designates the end of the
# previous step/substep.

class TreeStat:
    def __init__(self):
        self.current_section = ''
        self.steps_by_section = {}
        self.prev_data = None
        self.step_map = {}

    def get_current_section(self, dtime):
        if self.current_section in self.steps_by_section:
            return self.steps_by_section[self.current_section]

        print(f"Section not started.  Use default section")
        section = {
            'data': [],
            'start': dtime,
            'end': dtime,
        }
        self.steps_by_section[self.current_section] = section
        return section

    def start_section(self, name, dtime):
        self.current_section = name
        if name in self.steps_by_section:
            print(f"Section {name} already started")
            return

        section = {
            'data': [],
            'start': dtime,
            'end': dtime,
        }
        self.steps_by_section[self.current_section] = section
        self.step_map = {}

    def end_section(self, name, dtime):
        if name not in self.steps_by_section:
            print(f"Section {name} not started")
            return

        section = self.steps_by_section[name]
        section['end'] = dtime

        self.end_prev(dtime)

    def add_step(self, label, dtime):
        section = self.get_current_section(dtime)

        section['end'] = dtime

        if '::' in label:
            step, substep = label.split('::', 1)
            label = substep
            is_substep = True
        else:
            step = label
            is_substep = False

        data = {
            'step': step,
            'is_substep': is_substep,
            'label': label,
            'start': dtime,
            'end': None,
        }

        if not is_substep:
            self.step_map[step] = data

        self.end_prev(dtime)

        section['data'].append(data)
        self.prev_data = data

    def end_prev(self, dtime):
        if not self.prev_data:
            return

        prev_data = self.prev_data
        self.prev_data = None

        prev_data['end'] = dtime

        if prev_data['is_substep']:
            if prev_data['step'] in self.step_map:
                prev_step = self.step_map[prev_data['step']]
                prev_step['end'] = dtime
            else:
                print(f"Unexpected step {prev_data['step']} for substep {prev_data['label']}")
                steps = ', '.join(self.step_map.keys())
                print(f"  Known steps: {steps}")


class IndexerLogStdinPrinter:
    def __init__(self):
        self.stat_by_tree = {}

    def get_stat(self, tree):
        if tree in self.stat_by_tree:
            return self.stat_by_tree[tree]

        stat = TreeStat()
        self.stat_by_tree[tree] = stat
        return stat


    def consume(self, fp):
        for line in fp.readlines():
            m = re.match('(?:.+:)?Perform(ing|ed) (.+) (section|step) for (.+) : (.+)', line)
            if m is None:
                continue

            is_end = m.group(1) == 'ed'
            label = m.group(2)
            is_section = m.group(3) == 'section'
            tree = m.group(4)
            dtime = datetime.datetime.fromisoformat(m.group(5))

            stat = self.get_stat(tree)
            if is_section:
                if is_end:
                    stat.end_section(label, dtime)
                else:
                    stat.start_section(label, dtime)
                continue

            if is_end:
                print('"Performed" can be used only for sections.')
                continue

            stat.add_step(label, dtime)

    def print(self):
        root_tree = Tree('Trees')
        for tree_name, stat in self.stat_by_tree.items():
            for section_name, section in stat.steps_by_section.items():
                section_tree = root_tree.add(tree_name + ": " + section_name)

                table = Table(box=box.SIMPLE)
                table.add_column('step')
                table.add_column('time since start')
                table.add_column('apparent duration')

                for data in section['data']:
                    label = data['label']
                    elapsed = f"{data['start'] - section['start']}"
                    if data['end'] is not None:
                        dur = f"{data['end'] - data['start']}"
                    else:
                        dur = ''
                    if data['is_substep']:
                        label = '  ' + label
                        dur = f'({dur})'
                    else:
                        dur = f' {dur}'

                    table.add_row(
                        label,
                        elapsed,
                        dur,
                    )

                label = '(end)'
                elapsed = str(section['end'] - section['start'])
                dur = ''
                table.add_row(
                    label,
                    elapsed,
                    dur,
                )

                section_tree.add(table)
        print(root_tree)

if __name__ == '__main__':
    install(show_locals=True)
    cmd = IndexerLogStdinPrinter()
    cmd.consume(sys.stdin)
    cmd.print()
