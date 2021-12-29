#!/usr/bin/env python3

from trigger_common import TriggerCommandBase

# Usage: trigger_indexer.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>

# Note that this class is also used by `build-lambda-indexer-start.sh`'s
# dynamically generated hard-coded python script thing.
class TriggerIndexerCommand(TriggerCommandBase):
    def __init__(self):
        super().__init__('indexer', 'index.sh', 10)

    def script_args_after_branch_and_channel(self, args):
        return '''"{mozsearch_repo}" "{config_repo}" config "{config_input}"'''.format(
            mozsearch_repo=args.mozsearch_repo,
            config_repo=args.config_repo,
            config_input=args.config_input
        )

if __name__ == '__main__':
    cmd = TriggerIndexerCommand()
    cmd.parse_args()
    cmd.trigger()
