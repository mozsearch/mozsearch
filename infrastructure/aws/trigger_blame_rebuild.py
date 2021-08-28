#!/usr/bin/env python3

from trigger_common import TriggerCommandBase

# Usage: trigger_blame_rebuild.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>
#  e.g.: trigger_blame_rebuild.py https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla config1.json master release

class TriggerReblameCommand(TriggerCommandBase):
    def __init__(self):
        timeout_hours = 7 * 24 # upper bound on how long we expect the blame-rebuild to take
        super().__init__('blame-builder', 'rebuild-blame.sh', timeout_hours)

    def script_args_after_branch_and_channel(self, args):
        return '''config "{config_input}"'''.format(
            mozsearch_repo=args.mozsearch_repo,
            config_repo=args.config_repo,
            config_input=args.config_input
        )

if __name__ == '__main__':
    cmd = TriggerReblameCommand()
    cmd.parse_args()
    cmd.trigger()
