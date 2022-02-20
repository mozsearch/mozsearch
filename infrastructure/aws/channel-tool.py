#!/usr/bin/env python3

# Elastic Load Balancer (ELB) support code to support dynamic use of ELB
# instances for dev channels so that we don't need to leave them existing all
# the time, as there is a carrying cost to them.
#
# The general desired workflow is:
# - trigger-web-server.py creates a dev channel on demand if it doesn't already
#   exist.  This includes doing everything documented at
#   https://github.com/mozsearch/mozsearch/blob/master/docs/aws.md#creating-additional-development-channels
#   automatically.
# - terminate_indexer.py destroys a dev channel by default when terminating a
#   web-server.
#
# Probably fine logistical details:
# - The ELB gets a random internal DNS name which means that we have to
#   dynamically update the subdomain even if it already existed.  This means
#   that it's preferable for us to delete DNS entries when not using a channel.
#
# Disclaimers:
# - I'm really struggling naming-wise in python, using a very inconsistent
#   mixture of snake_case and camelCase.  Sorry!


import argparse
from datetime import datetime
import re
import sys

import boto3
from rich import box, print
from rich.console import Console
from rich.markup import escape
from rich.table import Table
from rich.tree import Tree
from rich.traceback import install

console = Console()

# These are documented in the `HelperResult` doc-block below.
HANDS_OFF_RELEASE_CHANNELS = 1
NO_SUCH_CHANNEL = 2
NO_SUCH_INSTANCE = 3
WEIRD_CHANNEL = 4

class HelperResult:
    '''
    Hacky attempt for the helper to provide suggested error codes and maybe some
    additional details, even though the helper will print out a tremendous
    amount of information itself.

    We probably should be using exceptions instead.

    Error codes:
    - 1: You're trying to mess with a release channel and we don't like that.
    - 2: You named a channel that doesn't exist and a channel does need to
         exist.
    - 3: You named an instance that doesn't exist.
    - 4: The channel's state is inconsistent and so can't be used for the
         requested task.  It's probably appropriate to run the "add" command
         again.
    '''
    def __init__(self, errcode=0):
        self.errcode = errcode

class ChannelHelper:
    '''
    Logic for dealing with our application load balancers via the elastic load
    balancer client, plus any associated DNS setup.
    '''
    def __init__(self):
        self.ec2_resource = boto3.resource('ec2')
        self.ec2 = boto3.client('ec2')
        self.elb = boto3.client('elbv2')
        self.dns = boto3.client('route53')

        ## State populated as a side-effect of calls to `determine_channels`.

        # Dict of channels by name.
        #
        # self.channels will be directly returned from the call above; this is
        # mainly intended as a convenience for hacky REPL-based debugging.
        self.channels = None


        # Dict of instances by id.
        #
        # This is not directly returned, and so this is a totally legitimate way
        # of getting at this information.  Not sketchy!
        self.instances = None

        self.zoneId = None

    def determine_channels(self):
        channels = self.channels = {}
        instances = self.instances = {}
        now = None

        def get_channel(name):
            channel = channels.get(name, None)
            if channel is None:
                if name.startswith('release'):
                    kind = 'release'
                else:
                    kind = 'development'
                channel = {
                    'name': name,
                    'kind': kind,
                    'instances': [],
                    'activeInstanceId': None,
                    # what components of a channel did we find?
                    # - lb: load balancer
                    # - tg: target group
                    # - dns: domain name info
                    'pieces': [],
                }
                channels[name] = channel
            return channel

        # Most of the instance logic is from ssh.py and should potentially be
        # extracted out into another method on this instance.  ssh.py should
        # likely just use this new class though since it can determine
        # interesting stuff about the channels
        for instance in self.ec2_resource.instances.all():
            # ssh.py did this for some reason
            if len(instance.security_groups) != 1:
                continue

            state = instance.state['Name']

            group = instance.security_groups[0]['GroupName']
            is_indexer = group.startswith('indexer')

            tags = {}
            if instance.tags:
                for tag in instance.tags:
                    tags[tag['Key']] = tag['Value']

            # datetime.now() is timezone-naive which means if we try and subtract
            # to get a timedelta without a tz, we'll get an error.  Since under
            # Python2 it's a little annoying to get the UTC timezone, we steal it.
            if now is None:
                now = datetime.now(instance.launch_time.tzinfo)
            age = now - instance.launch_time
            age_str = str(age)
            # strip off sub-seconds
            age_str = age_str[:age_str.find('.')]


            irep = instances[instance.id] = {
                'id': instance.id,
                'state': state,
                'group': group,
                'tags': tags,
                'age': age,
                'age_str': age_str,
                # Assume the instance is a spare unless we see it active in a
                # target group.
                'role': 'indexer' if is_indexer else 'spare',
                'target_groups': [],
            }

            channel = get_channel(tags.get('channel', 'None'))
            channel['instances'].append(irep)

        # The sub-domains live as "resource record sets" under the
        # "searchfox.org." host zone.
        hr = self.dns.list_hosted_zones()
        zoneId = None
        for zone in hr['HostedZones']:
            if zone['Name'] == 'searchfox.org.':
                zoneId = zone['Id']
                # normalize off any preceding `/hostedzone/`; not sure what's up
                # with that.
                RE_ZONE_ID = re.compile('^/hostedzone/(.+)$')
                m = RE_ZONE_ID.match(zoneId)
                if m:
                    zoneId = m.group(1)
                self.zoneId = zoneId

        if zoneId is not None:
            RE_SUB = re.compile('^([^.]+)\.searchfox\.org\.$')
            dr = self.dns.list_resource_record_sets(HostedZoneId=zoneId)
            for rrset in dr['ResourceRecordSets']:
                # We only care about searchfox.org subdomains
                m = RE_SUB.match(rrset['Name'])
                if m is None or rrset['Type'] != 'A':
                    continue
                name = m.group(1)
                channel = get_channel(name)
                channel['zoneId'] = zoneId
                channel['rrset'] = rrset
                channel['pieces'].append('dns')

        lbr = self.elb.describe_load_balancers()
        RE_LB_NAME = re.compile('^([^-]+)-lb$')
        for balancer in lbr['LoadBalancers']:
            lbName = balancer['LoadBalancerName']
            m = RE_LB_NAME.match(lbName)
            if m is None:
                continue
            name = m.group(1)
            channel = get_channel(name)
            lbArn = balancer['LoadBalancerArn']
            channel['loadBalancerArn'] = lbArn
            channel['pieces'].append('lb')

        RE_TARGET = re.compile('^([^-]+)-target$')
        tgr = self.elb.describe_target_groups()
        for tgroup in tgr['TargetGroups']:
            m = RE_TARGET.match(tgroup['TargetGroupName'])
            if m is None:
                continue
            name = m.group(1)
            channel = get_channel(name)
            tgArn = channel['targetGroupArn'] = tgroup['TargetGroupArn']
            if tgArn:
                channel['pieces'].append('tg')

            tg_health = self.elb.describe_target_health(TargetGroupArn=tgArn)
            for targetInfo in tg_health['TargetHealthDescriptions']:
                targetId = targetInfo['Target']['Id']
                inst = instances[targetId]
                inst['role'] = 'active'
                inst['target_groups'].append(name)
                channel['activeInstanceId'] = targetId

        ### Compute Channel Info
        for name, channel in channels.items():
            # Sort the instances by age.
            channel['instances'].sort(key=lambda x: x['age'])

        return channels

    def format_channels(self, channels):
        '''
        Given channel data from `determine_channels`, build a rich Tree suitable
        for (rich) print()ing.
        '''
        release_channels = []
        dev_channels = []

        for name, channel in channels.items():
            if channel['kind'] == 'release':
                release_channels.append(channel)
            else:
                dev_channels.append(channel)

        tree = Tree('Channels')

        def colorize_state(state):
            if state == 'stopped':
                return '[red]stopped[/red]'
            return state

        def populate_node_with_instances(node, instances):
            if len(instances) == 0:
                return

            # Build up a table to describe the list of instances so we get a
            # grid/table visual layout.
            table = Table(box=box.SIMPLE)

            # build up all known tags first
            tag_keys = ['(start time)']
            for instance in instances:
                for key in instance['tags'].keys():
                    # suppress tags we've already handled
                    # - channel: already a grouping heuristic
                    # - web-server/indexer: these encode the start time and
                    #   for table purposes it's preferable to fold them, but it
                    #   is misleading.
                    if key == 'channel' or key == 'web-server' or key == 'indexer':
                        continue
                    if key not in tag_keys:
                        tag_keys.append(key)
            tag_keys.sort()

            # populate the table headers
            table.add_column('role')
            table.add_column('target_groups')
            table.add_column('id')
            table.add_column('state')
            table.add_column('group')
            for key in tag_keys:
                table.add_column(key)


            # Instances as table rows
            for instance in instances:
                cells = [
                    instance['role'],
                    ','.join(instance['target_groups']),
                    instance['id'],
                    colorize_state(instance['state']),
                    instance['group'],
                ]
                # NB: It's possible the instance has keys the source of the keys
                # did not.
                for key in tag_keys:
                    if key == '(start time)':
                        if instance['role'] == 'indexer':
                            key = 'indexer'
                        else:
                            key = 'web-server'
                    value = instance['tags'].get(key, None)
                    if value and key.endswith('repo'):
                        value = value.replace('https://github.com/', '')
                    cells.append(value)
                table.add_row(*cells)

            node.add(table)


        def populate_node_with_channels(node, chans):
            chans.sort(key=lambda x: x['name'])
            for channel in chans:
                node_name = f"{channel['name']}: ({', '.join(channel['pieces'])})"
                chan_node = node.add(node_name)
                populate_node_with_instances(chan_node, channel['instances'])

        populate_node_with_channels(tree.add("Release"), release_channels)
        populate_node_with_channels(tree.add("Development"), dev_channels)

        return tree

    def ensure_channel(self, name):
        '''
        Ensure that all the pieces of a channel exist; intended as an idempotent
        version of adding a channel.
        '''
        if name.startswith('release'):
            print('[red]Nope! Release channels need to be manually configured![/red]')
            print('(Release channels share a common load balancer.)')
            return HelperResult(HANDS_OFF_RELEASE_CHANNELS)

        all_channels = self.determine_channels()

        channel = all_channels.get(name, None)
        if channel:
            pieces = channel['pieces']
        else:
            # define an empty channel so we can call get with a None default.
            channel = {}
            pieces = []

        print(f'Channel {name} currently has the following pieces existing:', pieces)
        print()

        ## Load Balancer ##
        console.rule(f'Ensuring "{name}" Load Balancer')
        lbArn = channel.get('loadBalancerArn', None)
        lbInfo = None
        if lbArn:
            print('Reusing load balancer with ARN:', escape(lbArn))
            lbr = self.elb.describe_load_balancers(LoadBalancerArns=[lbArn])
            lbInfo = lbr['LoadBalancers'][0]
        else:
            # Get the list of subnets for the availability zones in our region,
            # as we want the LB to cover all of them.
            snr = self.ec2.describe_subnets()
            use_subnets = []
            for subnet in snr['Subnets']:
                use_subnets.append(subnet['SubnetId'])

            # Get the 'load-balancer' security group id
            sgr = self.ec2.describe_security_groups(GroupNames=['load-balancer'])
            security_group_id = sgr['SecurityGroups'][0]['GroupId']

            lb_name =f'{name}-lb'
            cr = self.elb.create_load_balancer(
                Name=lb_name,
                Subnets=use_subnets,
                SecurityGroups=[security_group_id],
                Scheme='internet-facing',
                # no tags for now
                Type='application',
                # Confusingly, we use a "dualstack."-prefixed DNS, but we don't
                # want to pass "dualstack" here (which means IPv4 and IPv6)
                # because then it gets upset about our subnets not having an
                # IPv6 CIDR block.
                IpAddressType='ipv4',
            )

            print('Load Balancer Creation result:', cr)
            lbInfo = cr['LoadBalancers'][0]
            lbArn = lbInfo['LoadBalancerArn']
        lbDNS = lbInfo['DNSName']
        lbVpcId = lbInfo['VpcId']
        lbZoneId = lbInfo['CanonicalHostedZoneId']

        ## DNS ##
        console.rule(f'Ensuring "{name}" DNS')

        # The web UI puts this on for us, but it's not clear the API does.
        if not lbDNS.startswith('dualstack.'):
            lbDNS = 'dualstack.' + lbDNS
        print('Want to use Load Balancer DNS:', lbDNS)

        # Mention the state of the existing record for context
        existing_rrset = channel.get('rrset', None)
        if existing_rrset:
            print('Updating existing DNS RRSet', existing_rrset)
        else:
            print('No existing record, creating a new one.')

        # But we actually just do an UPSERT
        new_rrset = {
            'Name': f'{name}.searchfox.org.',
            'Type': 'A',
            'AliasTarget': {
                # the target needs to use the load-balancer's zone id, which is
                # very explicitly different from our searchfox.org zone id.
                'HostedZoneId': lbZoneId,
                'DNSName': lbDNS,
                'EvaluateTargetHealth': False
            },
        }

        dr = self.dns.change_resource_record_sets(
                    HostedZoneId=self.zoneId,
                    ChangeBatch={
                        'Comment': f"upsert channel {name}",
                        'Changes': [{
                            'Action': 'UPSERT',
                            'ResourceRecordSet': new_rrset,
                        }],
                    })
        print('DNS Upsert result:', dr)

        ## Target Group ##
        tgName = f'{name}-target'
        console.rule(f'Ensuring Target Group tgName')
        tgArn = channel.get('targetGroupArn', None)
        if tgArn:
            # The VpcId is specific to the load balancer, but we really should
            # not be in a situation where we had a target group but not the
            # corresponding load balancer.  So we'll just throw an error if
            # there is a mismatch.
            tgr = self.elb.describe_target_groups(LoadBalancerArn=lbArn)
            if len(tgr['TargetGroups']) == 0:
                print('[red]The target group is not associated with the balancer![/red]')
                print('This can happen if the add process failed partway through.')
                print('Please use the "remove" command to totally remove the channel,')
                print('then you can call this command again!')
                return HelperResult(WEIRD_CHANNEL)
            tgInfo = tgr['TargetGroups'][0]
            if tgInfo['VpcId'] != lbVpcId:
                print('[red]Target Group VpcId does not match Balancer VpcId![/red]')
                print('Please use the "remove" command to totally remove the channel,')
                print('then you can call this command again!')
                return HelperResult(WEIRD_CHANNEL)

            print('Reusing target group with ARN:', escape(tgArn))
        else:
            print('Creating new target group.')
            tgr = self.elb.create_target_group(
                Name=tgName,
                # we talk to the instances unencrypted over HTTP
                Protocol='HTTP',
                # This is the default and what we've been using; it might make
                # sense to change this in the future.
                ProtocolVersion='HTTP1',
                Port=80,
                VpcId=lbVpcId,
                # We just use whatever the defaults are for all the health
                # check settings right now as we don't even do sane things with
                # the health check and disable it on the instances.
                #
                # instance is the default and the right choice for this
                TargetType='instance',
                # We didn't previously create tags, but I figure why not?
                # Maybe this will help in the AWS Web UI.
                Tags=[{
                    'Key': 'channel',
                    'Value': name,
                }],
            )
            print('Target Group creation result:', tgr)
            tgArn = tgr['TargetGroups'][0]['TargetGroupArn']

        ## Listeners (HTTP and HTTPS) ##
        console.rule('Ensuring Listeners and their Rules')
        needed_listeners = {
            'HTTP': {
                'Protocol': 'HTTP',
                'Port': 80,
            },
            'HTTPS': {
                'Protocol': 'HTTPS',
                'Port': 443,
                # We just hardcode this because it probably won't change?
                'Certificates': [{
                    'CertificateArn': 'arn:aws:acm:us-west-2:653057761566:certificate/f40d4a04-a58b-4b19-a1e2-daaaa70abc43'
                }],
                # This is the default and therefore what we've been using, but
                # not exactly modern best practices.
                'SslPolicy': 'ELBSecurityPolicy-2016-08',
            }
        }
        lir = self.elb.describe_listeners(LoadBalancerArn=lbArn)
        for listener in lir['Listeners']:
            print('Reusing listener for protocol:', listener['Protocol'])
            needed_listeners.pop(listener['Protocol'])
        for protocol, mix_params in needed_listeners.items():
            print(f'Creating {protocol} listener')
            clr = self.elb.create_listener(
                LoadBalancerArn=lbArn,
                DefaultActions=[{
                    'TargetGroupArn': tgArn,
                    'Type': 'forward',
                }],
                **mix_params,
            )
            print(f'Created {protocol} listener:', clr)

        ## Done!
        console.rule('Done!')
        return HelperResult()


    def remove_channel(self, name):
        '''
        Attempt to remove a channel, printing progress output as we go and
        returning a boolean that indicates whether we think we removed the
        channel.
        '''
        if name.startswith('release'):
            print('[red]This tool does not mess with release channels![/red]')
            return HelperResult(HANDS_OFF_RELEASE_CHANNELS)

        all_channels = self.determine_channels()

        channel = all_channels.get(name, None)
        if channel is None:
            print('[red]No such channel[/red]:', name)
            return HelperResult(NO_SUCH_CHANNEL)

        lbArn = channel.get('loadBalancerArn', None)
        if lbArn:
            print('Located channel load balancer ARN:', escape(lbArn))

            # Per docs this also deletes the associated listeners (and their
            # rules).
            r = self.elb.delete_load_balancer(LoadBalancerArn=lbArn)
            print('Deleted load balancer (and listeners)!', r)
        else:
            print('[yellow]There was no load balancer to delete[/yellow]')

        tgArn = channel.get('targetGroupArn', None)
        if tgArn:
            print('Located channel target group ARN:', escape(tgArn))

            r = self.elb.delete_target_group(TargetGroupArn=tgArn)
            print('Deleted target group!', r)
        else:
            print('[yellow]There was no target group to delete[/yellow]')

        rrset = channel.get('rrset')
        if rrset:
            print('[green]Located channel subdomain:[/green]', rrset)

            r = self.dns.change_resource_record_sets(
                    HostedZoneId=channel['zoneId'],
                    ChangeBatch={
                        'Comment': f"remove channel {name}",
                        'Changes': [{
                            'Action': 'DELETE',
                            'ResourceRecordSet': rrset,
                        }],
                    })
            print('[green]Deleted subdomain:[/green]', r)
        else:
            print('[yellow]There was no DNS sub-domain to delete[/yellow]')

        return HelperResult()

    def inspect_channel(self, name):
        '''
        Dump detailed information about a channel's configuration.  This is
        primarily intended as a development tool.
        '''
        all_channels = self.determine_channels()
        channel = all_channels.get(name, None)
        if channel is None:
            print('[red]No such channel[/red]:', name)
            return HelperResult(NO_SUCH_CHANNEL)

        ## Dump non-channel-specific context
        ### Subnets (we need this for load balance creation)
        snr = self.ec2.describe_subnets()
        print('Context: Subnets:', snr)

        sgr = self.ec2.describe_security_groups()
        print('Context: Security Groups:', sgr)

        ## Dump channel-specific info
        lbArn = channel.get('loadBalancerArn', None)
        if lbArn:
            lbr = self.elb.describe_load_balancers(LoadBalancerArns=[lbArn])
            print('Load Balancer Description:', lbr)

            lir = self.elb.describe_listeners(LoadBalancerArn=lbArn)

            for listener in lir['Listeners']:
                listenerArn = listener['ListenerArn']
                print('Listener:', listener)

                rr = self.elb.describe_rules(ListenerArn=listenerArn)
                rules = rr['Rules']
                print('Listener Rules:', rules)

            tgr = self.elb.describe_target_groups(LoadBalancerArn=lbArn)
            print('Target Group Description:', tgr)

        rrset = channel.get('rrset')
        if rrset:
            print('DNS RRSet:', rrset)

        return HelperResult()

    def move_server(self, server_id, name):
        '''
        This method allows subclasses to contribute arguments to their script.ting (web) server to be the active server on a channel.
        This could actually be the same channel the server is already part of,
        but where the server is currently a spare and not the active server.

        This will (potentially) change the channel tag of the instance.

        This can result in up to 2 targets being de-registered:
        1. Any existing target on the target channel.
        2. The instance that's being moved _if it was active_.

        And then this will result in this instance being registered.
        '''
        channels = self.determine_channels()
        channel = channels.get(name, None)

        if channel is None:
            print('[red]No such channel:[/red]', name)
            # typo's are very likely
            print('Run the "add" command first if this was not a typo.')
            return HelperResult(NO_SUCH_CHANNEL)
        if 'targetGroupArn' not in channel:
            print(f'[red]Channel [white]{name}[/white] is not associated with a load balancer![/red]')
            return HelperResult(WEIRD_CHANNEL)

        instance = self.instances.get(server_id, None)
        if instance is None:
            print('[red]No such instance:[/red]', server_id)
            return HelperResult(NO_SUCH_INSTANCE)

        old_channel_name = instance['tags'].get('channel', None)
        old_channel = channels.get(old_channel_name, None)
        if old_channel is None:
            print('[yellow]The old channel does not exist?  Weird.[/yellow]')
        else:
            if old_channel_name in instance['target_groups']:
                tgArn = old_channel.get('targetGroupArn', None)
                if tgArn is None:
                    print('[yellow]No old target group ARN, cannot deregister.[/yellow]')
                else:
                    print('Deregistering server from old target group.')
                    dr = self.elb.deregister_targets(TargetGroupArn=tgArn,
                                                     Targets=[{'Id': server_id, 'Port': 80 }])
                    print('Deregistered:', dr)

        activeInstanceId = channel.get('activeInstanceId')
        tgArn = channel['targetGroupArn']
        if activeInstanceId:
            print('Deregistering currently active server from new channel:', activeInstanceId)
            dr = self.elb.deregister_targets(TargetGroupArn=tgArn,
                                             Targets=[{'Id': activeInstanceId, 'Port': 80 }])
            print('Deregistered:', dr)

        print('Updating channel tag from', old_channel_name, 'to', name)
        # create_tags will over-write existing tags
        ctr = self.ec2.create_tags(
            Resources=[server_id],
            Tags=[{
                'Key': 'channel',
                'Value': name,
            }])
        print('Updated tag:', ctr)

        print('Registering server as target for requested channel')
        rtr = self.elb.register_targets(
            TargetGroupArn=tgArn,
            Targets=[{
                'Id': server_id,
                'Port': 80,
            }])
        print('Registered:', rtr)

        print('Done!  But note that it can take some time for the target groups to update!')
        return HelperResult()


class ChannelCommand:
    '''
    Shallow exposure of the ChannelHelper logic on the command line.  The intent
    is that the ChannelHelper can be used directly by other scripts, so there
    should be no meaningful application logic here, just parsing and glue.
    '''
    def __init__(self):
        self.args = None

    def make_parser(self):
        parser = argparse.ArgumentParser()
        parser.add_argument('--verbose', '-v', action='count', default=0)

        subparsers = parser.add_subparsers()

        list_parser = subparsers.add_parser('list', help='List active and possible channels.')
        inspect_parser = subparsers.add_parser('inspect', help='Show detail channel debug info.')
        #cleanup_parser = subparsers.add_parser('cleanup', help='Cleanup active but unused load balancers.')
        add_parser = subparsers.add_parser('add', help='Add a possible channel.')
        remove_parser = subparsers.add_parser('remove', help='Remove a channel.')
        move_server_parser = subparsers.add_parser('move-server', help='Move a server between channels and make it active.')

        list_parser.set_defaults(func=self.do_list)

        inspect_parser.add_argument('name')
        inspect_parser.set_defaults(func=self.do_inspect)

        #cleanup_parser.set_defaults(func=self.do_cleanup)

        add_parser.add_argument('name')
        add_parser.set_defaults(func=self.do_add)

        remove_parser.add_argument('name')
        remove_parser.set_defaults(func=self.do_remove)

        move_server_parser.add_argument('server_id')
        move_server_parser.add_argument('channel_name')
        move_server_parser.set_defaults(func=self.do_move_server)

        return parser

    def parse_args(self):
        self.parser = self.make_parser()
        self.args = self.parser.parse_args()

    def run(self):
        if 'func' not in self.args:
            self.parser.print_help()
            return

        self.helper = ChannelHelper()
        result = self.args.func(self.helper, self.args)
        sys.exit(result.errcode)

    def do_list(self, helper, args):
        channels = helper.determine_channels()
        print(helper.format_channels(channels))
        return HelperResult()

    def do_inspect(self, helper, args):
        return helper.inspect_channel(args.name)

    def do_cleanup(self, helper, args):
        pass

    def do_add(self, helper, args):
        return helper.ensure_channel(args.name)

    def do_remove(self, helper, args):
        return helper.remove_channel(args.name)

    def do_move_server(self, helper, args):
        return helper.move_server(args.server_id, args.channel_name)


if __name__ == '__main__':
    install(show_locals=True)
    cmd = ChannelCommand()
    cmd.parse_args()
    cmd.run() # calls sys.exit and never returns
