#!/usr/bin/env python3

from trigger_common import TriggerCommandBase

# Usage: trigger_indexer.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>


class TriggerShellCommand(TriggerCommandBase):
    def __init__(self):
        max_hours = 6
        super().__init__('shell', 'shell-setup.sh', max_hours)

    def script_args_after_branch_and_channel(self, args):
        return '''"{mozsearch_repo}" "{config_repo}" config "{config_input}"'''.format(
            mozsearch_repo=args.mozsearch_repo,
            config_repo=args.config_repo,
            config_input=args.config_input
        )

if __name__ == '__main__':
    cmd = TriggerShellCommand()
    cmd.parse_args()
    cmd.trigger()
