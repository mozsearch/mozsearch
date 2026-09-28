#!/usr/bin/env python3

from trigger_common import TriggerCommandBase

# Usage: trigger_blame_rebuild.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>
#  e.g.: trigger_blame_rebuild.py https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla config1.json master release
# See "Rebuilding blame and history" in docs/aws.md, and reblame-status.py for
# checking on the instance.

class TriggerReblameCommand(TriggerCommandBase):
    def __init__(self):
        timeout_hours = 7 * 24 # upper bound on how long we expect the blame-rebuild to take
        super().__init__('blame-builder', 'rebuild-blame.sh', timeout_hours)

    def make_parser(self):
        parser = super().make_parser()
        # Rebuilds are usually of branches whose tools may not be in the binary
        # cache yet, and a bigger root volume costs little for their duration.
        parser.set_defaults(root_volume_gb=100)
        return parser

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
