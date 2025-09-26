# Note that any 3rd party lib dependencies will also need to be added to the
# install steps in `build-lambda-indexer-start.sh`.
import boto3
import argparse
from datetime import datetime, timedelta
import sys
import os.path

class TriggerCommandBase:
    '''
    Helper class for launching indexers to extract out the commonality.  This
    doesn't need to get particularly complex.

    The general idea for now is:
    - If you have a new high-level action that wants to trigger an indexer,
      create a new script like `trigger_indexer.py` that's a thin wrapper
      around this class/module.
    - Place logic that can be used across all VM variants in here, or specialize
      in the subclass as appropriate.
    '''
    def __init__(self, indexer_type, core_script, max_runtime_hours):
        self.indexer_type = indexer_type
        self.core_script = core_script
        self.max_runtime_hours = max_runtime_hours

        self.args = None

    def make_parser(self):
        parser = argparse.ArgumentParser()
        parser.add_argument('mozsearch_repo')
        parser.add_argument('config_repo')
        parser.add_argument('config_input')
        parser.add_argument('branch')
        parser.add_argument('channel')

        parser.add_argument('--verbose', '-v', action='count', default=0)

        parser.add_argument('--setenv', dest='env_vars', action='append', default=[])

        # Allow specifying a specific revision for the configuration repo; by
        # default we use the "branch" specified above, but if that branch
        # doesn't exist in the config repo, currently we'll break because we do
        # a shallow clone.  Previously we'd failover to the "master" branch, but
        # that was lost.  And for backwards-compat reasons I don't want to make
        # this a mandatory arg right now.  But arguably we should either:
        # - If this arg isn't provided, check if the "branch" exists on the
        #   server in this script so we can fail over to the 'master' branch if
        #   not.
        # - Teach `infrastructure/indexer.provision.sh` to handle us giving it a
        #   bad CONFIG_REV by falling back to the 'master' branch, but that
        #   requires a re-provision so I'm punting.
        parser.add_argument('--config-rev', dest='config_rev')

        return parser

    def parse_args(self, args=None):
        parser = self.make_parser()
        self.args = parser.parse_args(args)

    def script_args_after_branch_and_channel(self, args):
        '''
        This method allows subclasses to contribute arguments to their script.
        Note that all scripts will be provided with the branch and channel as
        their first two arguments because `main.sh` needs this information and
        assumes those arguments exist.

        This method's return value is interpolated directly into the bash shell
        script built by `trigger` without any escaping.  This means arguments
        should probably be quoted and escaped as appropriate.
        '''
        return ""

    def build_extra_commands(self, args):
        '''
        Builds a single command-string, including newlines, that will be
        inserted into the bash shell script built by `trigger`.
        '''
        cmds = []

        for setenv in args.env_vars:
            cmds.append('export ' + setenv)

        return "\n".join(cmds)

    def trigger(self):
        if self.args is None:
            raise Exception('Arguments were not parsed first!')
        args=self.args
        extra_args = self.script_args_after_branch_and_channel(args)

        extra_commands = self.build_extra_commands(args)

        ec2 = boto3.resource('ec2')
        client = boto3.client('ec2')

        # Indexers that want more powerful instance:
        # - release4 (bug 1922407); runtimes have hit and timed out at 12 hours
        #   using an m5d.2xlarge
        # - release5 (bug 1912078 ish): runtime hit 8.5 hours and much of this
        #   is simply build duration for webkit, so should parallelize easily.
        #
        # This decision is baked into the script here rather than present in
        # config files because we run this script as part of a lambda job run
        # out of a zipball we upload to AWS without doing any git checkouts,
        # etc.  The git stuff happens on the indexer after it is spawned.  (This
        # could of course be changed, but potentially would make the lambda jobs
        # more complex / brittle.)
        if args.channel == "release4" or args.channel == "release5":
            instance_type = 'm6id.4xlarge'
        else:
            instance_type = 'm6id.2xlarge'

        # Terminate any "running" or "stopped" instances.  We used to only
        # terminate "running" instances with the theory that someone might get
        # around to investigating the "stopped" instance, but the reality is
        # that:
        # - Frequently failures are one-offs that have an obvious cause in the
        #   emailed log.  And we can provide more log context!
        # - If someone is going to look at the problem, they can usually decide
        #   to do that before the next indexer run.  The investigation doesn't
        #   need to complete, the indexer just needs to be re-tagged to not look
        #   like an indexer.  Currently this would require using the EC2 console
        #   but this can easily be added to `channel-tool.py`.
        instances = ec2.instances.filter(Filters=[{'Name': 'tag-key', 'Values': [self.indexer_type]},
                                            {'Name': 'tag:channel', 'Values': [args.channel]},
                                            {'Name': 'instance-state-name', 'Values': ['running', 'stopping', 'stopped']}])
        for instance in instances:
            print(f"Terminating existing {instance.state['Name']} {self.indexer_type} {instance.instance_id} for channel {args.channel}")
            instance.terminate()

        user_data = '''#!/usr/bin/env bash

    cd ~ubuntu
    {extra_commands}
    sudo -i -u ubuntu {cmd_env_vars} ./update.sh "{mozsearch_repo}" "{branch}" "{config_repo}" "{config_rev}"
    sudo -i -u ubuntu {cmd_env_vars} mozsearch/infrastructure/aws/main.sh {core_script} {max_runtime_hours} "{branch}" "{channel}" {extra_args}
    '''.format(
        core_script=self.core_script,
        max_runtime_hours=self.max_runtime_hours,
        branch=args.branch,
        channel=args.channel,
        mozsearch_repo=args.mozsearch_repo,
        config_repo=args.config_repo,
        config_rev=args.config_rev or args.branch,
        cmd_env_vars=" ".join(args.env_vars),
        extra_commands=extra_commands,
        extra_args=extra_args
        )

        block_devices = []

        # We only have "indexer" and "web-server" AMI types, and currently all
        # subclasses do want to be using an indexer AMI which is consistent with
        # our hardcoded choice of InstanceType and role, etc.
        images = client.describe_images(
            Owners=['self'],
            Filters=[{'Name': 'tag-key', 'Values': ['indexer']}]
        )
        image_id = images['Images'][0]['ImageId']

        launch_spec = {
            'ImageId': image_id,
            'KeyName': 'Main Key Pair',
            'SecurityGroups': ['indexer-secure'],
            'UserData': user_data,
            'InstanceType': instance_type,
            'BlockDeviceMappings': block_devices,
            'IamInstanceProfile': {
                'Name': 'indexer-role',
            },
            'TagSpecifications': [{
                'ResourceType': 'instance',
                'Tags': [{
                    'Key': self.indexer_type,
                    'Value': str(datetime.now())
                }, {
                    'Key': 'channel',
                    'Value': args.channel,
                }, {
                    'Key': 'branch',
                    'Value': args.branch,
                }, {
                    'Key': 'mrepo',
                    'Value': args.mozsearch_repo,
                }, {
                    'Key': 'crepo',
                    'Value': args.config_repo,
                }, {
                    'Key': 'cfile',
                    'Value': args.config_input,
                }],
            }],
        }

        if args.verbose > 0:
            print('Launch Spec:')
            print(repr(launch_spec))

        return client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
