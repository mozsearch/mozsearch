# AWS deployment

Mozsearch supports being deployed to AWS. Incoming requests are
handled by an Elastic Load Balancer instance. Using ELB means that the
web server machines don't need to know anything about the TLS
certificate being used. ELB takes care of that. The load balancer
directs each incoming request to a "target group", which consists
of a single EC2 machine that handles web serving. The target group
is chosen based on the code repository that the request is directed
to. As of this writing, for example, the mozilla-central repository
is handled by the "release-target" target group, while the mozilla-beta
repository is handled by the "mozilla-releases-target" target group.
The mapping from repository to target group is set manually by path
routing rules in the load balancer configuration.

An AWS Lambda task runs each day to start indexing of all the
trees. This job starts up EC2 instances to perform the indexing. Each
indexing instance takes care of the repos from a single config file.
So there will be one indexer instance processing the repos in
[config.json](https://github.com/mozsearch/mozsearch-mozilla/config.json)
and another instance processing the repos in
[mozilla-releases.json](https://github.com/mozsearch/mozsearch-mozilla/blob/master/mozilla-releases.json).
The indexing instances have an extra Elastic Block Store volume attached
where the index will be stored. The following paragraps explain the
lifecycle of a single indexer and its web server; the lifecycle applies
to each indexer instance.

The indexing instance downloads all the Git repositories from an
Amazon S3 bucket. It updates these repositories to the latest version
of the code. It also downloads the most recent blame repository from
an S3 bucket and incrementally builds blame revisions corresponding to
the new git revisions. When this is done, the updated repositories
(including blame) are uploaded back to the same S3 bucket. Following
that, the normal indexing process commences.

When indexing is complete, the indexer instance unmounts and detaches
the EBS volume containing the index. It does this using the AWS
API. Then the indexer uses the API to start a new EC2 instance for web
serving, passing it the name of the indexer instance as an
argument. The web server instance attaches and mounts the index volume
and starts serving web pages.

The indexer instance waits for the web server to report that it is
up and running (by polling the /status.txt URL, which is updated by
the web server processes). Then it updates the ELB
target group to point to the new web server instead of the old
one. Finally, it shuts down and destroys any old web server instances
and index volumes. Finally, the indexer instance terminates itself.

## Logging into the AWS console

The AWS console allows you to manually control AWS resources. To log
in, you need to request an IAM identity for the Searchfox
account. After you've logged in, you need to [change the AWS region in
the top right
corner](http://docs.aws.amazon.com/awsconsolehelpdocs/latest/gsg/getting-started.html#select-region). The
region for Searchfox is "US West (Oregon)". Now you should be able to
select EC2 from Services and see the list of EC2 machines running.

Web server instances use the t2.large instance type while indexers use
the c3.2xlarge type. When selecting an instance, the most important
data is the "Launch time" and "IPv4 Public IP".

## Setting up AWS locally

Mozsearch uses a lot of scripts that use the AWS API to start and stop
indexing, provision servers, etc. It's better to run these scripts
**outside** the VM so that you don't need to store credentials in the
VM (where they might get deleted easily).

To start, you'll need to create some AWS configuration files in your
home directory.

```
# RUN THESE COMMANDS OUTSIDE THE VM!

mkdir ~/.aws

cat > ~/.aws/config <<"THEEND"
[default]
region = us-west-2
THEEND

cat > ~/.aws/credentials <<"THEEND"
[default]
aws_access_key_id =
aws_secret_access_key =
THEEND
```

Now we need to fill in the keys for the latter file. To create an
access key, [follow the instructions for creating an access key from
the AWS
console](http://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_access-keys.html#Using_CreateAccessKey)
(you'll need to scroll down to the section labeled "To create, modify,
or delete a user's access keys"). Rather than downloading the
credentials for the new key, it's easier to copy the key ID and secret
key and paste them into the `~/.aws/credentials` file.

Once the credentials are set up, the AWS Python API must be
installed. First, create a virtual environment in the mozsearch git
repository. Then install the `boto3` package, which is the AWS Python
library.

```
# Run these commands from within a mozsearch checkout.

virtualenv env
source env/bin/activate
pip install boto3
```

All later AWS commands should be run within the virtual environment.

## SSHing into AWS machines

To SSH into an EC2 instance, you will need to obtain the private key
file for Searchfox. Once you have the key, ensure that the permissions
are set so that it is not world-readable. Put it (or a symlink to it) at
`~/.aws/private_key.pem`. The ssh script below will check this location
and use this as the identity file if it exists.

Now you can connect to an instance as follows:

```
python infrastructure/aws/ssh.py
```

This command will print a list of instances that you can connect to as
well as details about them. Select an instance ID (starting with `i-`)
and connect to it:

```
python infrastructure/aws/ssh.py i-955af89
```

## Lambda

The AWS Lambda task uses a cron-style scheduler to run once a day. The
task that runs can be generated by running the following command inside
the Vagrant VM instance:

```
./infrastructure/aws/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config.json \
  master \
  release
```

The first three arguments are links to the repositories containing
Mozsearch and the configuration to use. The fourth argument is a branch
name. When scripts check out Mozsearch or the configuration
repository, they will check out this branch. The last argument is used
to determine which ELB target group will be updated. The `release`
argument updates the `release-target` target group (which might
control, for example, `example.com`). The `dev` argument updates the
`dev-target` target group (which might control, for example,
`dev.example.com`).

When the script finishes, it generates a file, `/tmp/lambda.zip`, that
can be uploaded to AWS Lambda using the AWS control panel.

## Triggering indexing manually

It's fairly easy to trigger an indexing job manually from your local
computer. To do so, run the following from within the Python virtual environment:

```
python infrastructure/aws/trigger_indexer.py \
  https://github.com/some-user/mozsearch \
  https://github.com/some-user/mozsearch-mozilla \
  some-config.json \
  some-development-branch \
  dev
```

The arguments here are the same as those to
`build-lambda-indexer-start.sh`. In this example, the new index would
appear on `dev.example.com` and it would use Mozsearch code from the
`some-development-branch` branch.

Note that the .zip file created for AWS Lambda in the previous section
merely includes a copy of the `trigger_indexer.py` script, which it
invokes when the task runs.

## Creating additional channels

If many developers are working on features concurrently, it might be
useful to set up additional channels so they can test on AWS without
stepping on each others' toes. In order to create a channel, the
following steps need to be done in the AWS console:

1. Decide on a name for the new channel. These instructions will use
   `foo` as the name.
2. In the EC2 console, go to the Load Balancers section and create
   a new Load Balancer (of type Application Load Balancer). Give it
   a name like `foo-lb`.  The non-default settings needed are:
- Listeners: add listeners for both HTTP and HTTPS
- Availability Zones: select all three availability zones
- Certificate: use the wildcard certificate for `*.searchfox.org`
  from ACM.
- Security group: Select the load-balancer security group
- Target group: Create a new target group with name `foo-target`
3. After creating the new Load Balancer, copy the DNS name from
   the description tab (something like `foo-lb-123456789.us-west-2.elb.amazonaws.com`)
4. Go to the Route 53 console, and under the `searchfox.org` Hosted
   Zone, add a new Record Set with the following properties:
- Name: `foo` (it will append `.searchfox.org` automatically)
- Type: A - IPv4 address
- Alias: Yes
- Alias Target: the DNS name copied from the Load Balancer. Note that
  it will automatically prepend `dualstack.` to the name.

That's it! After this is set up, you can trigger an indexer run
using the `foo` channel (instead of `dev` or `release`) and it
will show up at https://foo.searchfox.org once it is complete.

## Provisioning and cloud init

The EC2 instances for indexing and web serving are started using a
custom Amazon Machine Image (AMI). This is the disk image used for
booting the machine. These AMIs are based off Ubuntu 16.04, but
additional software has been installed for all the basic dependencies,
like clang for the indexing machine and nginx for the web server.

The AMIs also contain the Ubuntu cloud init package, which allows a
custom shell script to be passed to the machines using the Amazon API
when the instances are created. The shell script runs after the
machine boots up. Mozsearch uses the shell script to pass in
parameters like the branch, channel, and configuration repository. The
`trigger_index.py` and `trigger-web-server.py` scripts generate the
custom shell scripts that are sent to indexing and web server
instances.

New AMIs need to be built every time a dependency changes (if a newer
version of Clang is required, for example). The first step is to edit
the provisioning scripts to change dependencies:

```
# Update dependencies for indexing...
nano infrastructure/indexer-provision.sh

# Update dependencies for web serving...
nano infrastructure/web-server-provision.sh
```

Generating a new AMI is still a somewhat manual process. To provision
the AMI for indexing, run the following from within the Python virtual
environment:

```
python infrastructure/aws/trigger-provision.py \
  infrastructure/indexer-provision.sh \
  infrastructure/aws/indexer-provision.sh
```

For web serving, use this command:

```
python infrastructure/aws/trigger-provision.py \
  infrastructure/web-server-provision.sh
```

The `trigger-provision.py` script starts a new EC2 instance and uses
cloud-init to run the given provisioner shell scripts in it. These
scripts install all the required dependencies. When the scripts finish
(which you need to check for manually by looking up the machine in the
AWS console, sshing into it, and `tail`ing the provision.log file to
check for completion), you can use the AWS console to generate an AMI from the
instance. Select the instance in the console, then choose "Actions,
Image, Create Image". The Image Name must be changed to
`indexer-16.04` or `web-server-16.04`. The other values can remain as
before. (Note: make sure to delete any old AMIs of the same name
before doing this.) Once the AMI is created, new jobs will use it
automatically.

## Updating the machine after startup

Some dependencies change too often to require a new image, so they are
installed every time an instance boots. These include Rust and
Spidermonkey, since the Gecko build process will fail if they are out
of date. Additionally, a current version of Mozsearch and the
configuration repository must be installed when each instance is
started.

The provisioner scripts automatically install a `~ubuntu/update.sh`
script that downloads/builds this software. This script is run by the
custom cloud-init script when each instance is started.

## Error handling

When the indexer instance is created, a crontab file is installed that
runs an error handling script after 6 hours. The presumption is that
any indexing job taking more than 6 hours must have failed. The error
handling script uses Amazon Simple Email Service to send an email
notifying the Searchfox email list that indexing failed. Then it shuts
down (but does not destroy) the EC2 indexer instance. The instance can
be debugged by starting it up again from the AWS console and logging
into it via ssh.

Even on successful runs, the index log is grepped for warning lines,
and an email is sent to the searchfox mailing list containing these
warnings. Warnings are "recoverable errors" in that the indexing completed
with a new deployment, but some part of the functionality may be missing
due to an error that needs fixing. The complete log is uploaded to
the `indexer-logs` S3 bucket, so if additional context is needed for the
warnings, you can download the complete log from there and inspect it.
The name of the log is the timestamp at completion, suffixed with the
channel (e.g. `release`) and the file stem of the config file used.

## Debugging errors

If an error occurs, the email sent to the searchfox mailing list will
contain some of the log output. The log in the email may make it obvious
what the root cause was. If not, you may have to start up the indexer
instance using the EC2 web console, and then SSH in to it to examine
the log in more detail and/or inspect other state to debug the problem.
After SSH'ing to the indexer, you should run the command:
```
sudo mount /dev/xvdf /index
```
to re-mount the data volume. This will allow you to inspect the state
on the data volume as well as run additional commands for debugging
purposes, or to test a fix. The shell scripts that run during indexing
generally require some environment variables to be set; you can set
up the main ones by sourcing the load-vars.sh script like so:
```
export MOZSEARCH_PATH=$HOME/mozsearch
# Replace the last two arguments with the appropriate config file
# and repo that errored out
source $MOZSEARCH_PATH/scripts/load-vars.sh $HOME/config/config.json mozilla-central
```

After the debugging is complete, or even if no SSHing is required,
it is important to terminate the indexer and delete the incomplete
index volume, otherwise they will sit around forever and eat up money.
You can terminate the indexer either through the EC2 web console, or
by running
```
python infrastructure/aws/terminate-indexer.py <instance-id>
python infrastructure/aws/delete-volume.py <volume-id>
```
from within your local searchfox virtualenv (see the above section
on setting up AWS locally). The terminate-indexer.py script or the
web console will let you know the volume ID of the volume to delete.
