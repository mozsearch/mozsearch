# AWS deployment

Mozsearch supports being deployed to AWS. Incoming requests are
handled by an Elastic Load Balancer instance. Using ELB means that the
web server machines don't need to know anything about the TLS
certificate being used. ELB takes care of that. The load balancer
directs requests to a "target group", which consists of a single EC2
machine that handles web serving.

An AWS Lambda task runs each day to start indexing of all the
trees. This job starts up an EC2 instance to perform the indexing. The
indexing instance has an extra Elastic Block Store volume attached
where the index will be stored.

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

The indexer instance verifies that the web server instance is
functioning normally using some smoke tests. Then it updates the ELB
target group to point to the new web server instead of the old
one. Finally, it shuts down and destroys any old web server instances
and index volumes. Finally, the indexer instance terminates itself.

## Lambda

The AWS Lambda task uses a cron-style scheduler to run once a day. The
task that runs is generated as follows:

```
/vagrant/infrastructure/aws/build-lambda-indexer-start.sh \
  https://github.com/bill-mccloskey/mozsearch-mozilla \
  master \
  release
```

The first argument is a link to the repository containing the
Mozsearch configuration to use. The second argument is a branch
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
computer. To do so, run the following from within the Vagrant VM:

```
python /vagrant/infrastructure/aws/trigger_indexer.py \
  https://github.com/bill-mccloskey/mozsearch-mozilla \
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
vi /vagrant/infrastructure/indexer-provision.sh

# Update dependencies for web serving...
vi /vagrant/infrastructure/web-server-provision.sh
```

Generating a new AMI is still a somewhat manual process. To provision
the AMI for indexing, run the following from a Vagrant VM:

```
python /vagrant/infrastructure/aws/trigger-provision.py \
  /vagrant/infrastructure/indexer-provision.sh \
  /vagrant/infrastructure/aws/indexer-provision.sh
```

For web serving, use this command:

```
python /vagrant/infrastructure/aws/trigger-provision.py \
  /vagrant/infrastructure/web-server-provision.sh
```

The `trigger-provision.py` script starts a new EC2 instance and uses
cloud-init to run the given provisioner shell scripts in it. These
scripts install all the required dependencies. When the scripts finish
(which you need to check for manually by looking up the machine in the
AWS console, sshing into it, and using `ps` to verify that nothing is
running), you can use the AWS console to generate an AMI from the
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
notifying the administrator that indexing failed.
