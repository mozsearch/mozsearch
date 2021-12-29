import boto3
import argparse
from datetime import datetime, timedelta
import sys
import os.path

# Usage: trigger_indexer.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>

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

        return parser

    def parse_args(self):
        parser = self.make_parser()
        self.args = parser.parse_args()

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

        running = ec2.instances.filter(Filters=[{'Name': 'tag-key', 'Values': [self.indexer_type]},
                                            {'Name': 'tag:channel', 'Values': [args.channel]},
                                            {'Name': 'instance-state-name', 'Values': ['running']}])
        for instance in running:
            print("Terminating existing running %s %s for channel %s" % (self.indexer_type, instance.instance_id, args.channel))
            instance.terminate()


        user_data = '''#!/usr/bin/env bash

    cd ~ubuntu
    {extra_commands}
    sudo -i -u ubuntu ./update.sh "{branch}" "{mozsearch_repo}" "{config_repo}"
    sudo -i -u ubuntu mozsearch/infrastructure/aws/main.sh {core_script} {max_runtime_hours} "{branch}" "{channel}" {extra_args}
    '''.format(
        core_script=self.core_script,
        max_runtime_hours=self.max_runtime_hours,
        branch=args.branch,
        channel=args.channel,
        mozsearch_repo=args.mozsearch_repo,
        config_repo=args.config_repo,
        extra_commands=extra_commands,
        extra_args=extra_args
        )

        block_devices = []

        # We only have "indexer" and "web-server" AMI types, and currently all
        # subclasses do want to be using an indexer AMI which is consistent with
        # our hardcoded choice of InstanceType and role, etc.
        images = client.describe_images(Filters=[{'Name': 'tag-key', 'Values': ['indexer']}])
        # TODO: sort/pick the highest datestamp-y "indexer" tag Value.
        image_id = images['Images'][0]['ImageId']

        launch_spec = {
            'ImageId': image_id,
            'KeyName': 'Main Key Pair',
            'SecurityGroups': ['indexer-secure'],
            'UserData': user_data,
            'InstanceType': 'm5d.2xlarge',
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

