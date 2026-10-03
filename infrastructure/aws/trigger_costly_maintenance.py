#!/usr/bin/env python3

from trigger_common import TriggerCommandBase

# Usage: trigger_costly_maintenance.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>
#  e.g.: trigger_costly_maintenance.py https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla config1.json master release
# Launches an instance which runs the costly-maintenance script of each tree in
# the config file which has one (ex: firefox-disco's full repack of its
# history), and then terminates; see "Costly maintenance" in docs/aws.md.
# Lambda jobs run this monthly (see build-lambda-costly-maintenance.sh), and
# reblame-status.py shows how the instance is doing.

class TriggerCostlyMaintenanceCommand(TriggerCommandBase):
    def __init__(self):
        timeout_hours = 24 # upper bound on how long we expect the maintenance to take
        super().__init__('costly-maintenance', 'costly-maintenance.sh', timeout_hours)

    def make_parser(self):
        parser = super().make_parser()
        # The maintenance is costly in memory (ex: fully repacking the full
        # firefox history's timeline, 246M objects, took ~50 GiB, besides the
        # packs it mapped) and disk (the history, and its repack, which needs
        # about as much space again), so it defaults to the instance type the
        # full firefox reblame did that on, with more swap than the 8 GiB
        # default (see mkscratch.sh), and the root volume reblame gets, for
        # building the tools if they aren't in the binary cache.
        parser.set_defaults(instance_type='m8id.8xlarge', root_volume_gb=100,
                            env_vars=['SWAP_GIB=64'])
        return parser

    def script_args_after_branch_and_channel(self, args):
        return '''config "{config_input}"'''.format(
            mozsearch_repo=args.mozsearch_repo,
            config_repo=args.config_repo,
            config_input=args.config_input
        )

if __name__ == '__main__':
    cmd = TriggerCostlyMaintenanceCommand()
    cmd.parse_args()
    cmd.trigger()
