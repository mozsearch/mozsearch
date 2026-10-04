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
        # The maintenance is costly in memory and disk, so it defaults to an
        # r6id.4xlarge: 128 GiB of memory, 16 CPUs, and a 950 GB SSD, for $1.21
        # an hour on demand in 2026-10 (an m8id.8xlarge, which has as much
        # memory, twice the CPUs and SSD, was $2.09, and an m6id.4xlarge, with
        # half the memory, $0.95).  Fully repacking the full firefox history's
        # timeline, 246M objects, took ~50 GiB of memory, which would leave a
        # 64 GiB instance little for caching the 130 GiB of packs it read, and
        # used one CPU most of the time (on the full firefox reblame's
        # m8id.8xlarge, which kept 70-90 GiB of the packs cached).  The
        # history (~120 GiB) and its repack (about as much space again) fit
        # on the SSD.  More swap than the 8 GiB default (see mkscratch.sh)
        # covers needing more memory than that, and the root volume reblame
        # gets is for building the tools if they aren't in the binary cache.
        parser.set_defaults(instance_type='r6id.4xlarge', root_volume_gb=100,
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
