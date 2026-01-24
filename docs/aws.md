# AWS deployment

Mozsearch supports being deployed to AWS. Incoming requests are
handled by an Elastic Load Balancer instance. Using ELB means that the
web server machines don't need to know anything about the TLS
certificate being used. ELB takes care of that. The load balancer
directs each incoming request to a "target group", which consists
of a single EC2 machine that handles web serving. The target group
is chosen based on the code repository that the request is directed
to. As of this writing, for example, the firefox-main repository
is handled by the "release1-target" target group, while the mozilla-beta
repository is handled by the "release2-target" target group.
The mapping from repository to target group is set manually by path
routing rules in the load balancer configuration.

AWS Lambda tasks run each day to start indexing of all the
trees. These jobs start up EC2 instances to perform the indexing. Each
indexing instance takes care of the repos from a single config file.
So there will be one indexer instance processing the repos in
[config1.json](https://github.com/mozsearch/mozsearch-mozilla/blob/master/config1.json),
another instance processing the repos in
[config2.json](https://github.com/mozsearch/mozsearch-mozilla/blob/master/config2.json),
etc. The indexing instances have an extra Elastic Block Store volume attached
where the index will be stored. The following paragraphs explain the
lifecycle of a single indexer and its web server; the lifecycle applies
to each indexer instance.

Note that as of this writing, config1.json, config2.json, config4.json, and config5.json
are processed via the above-described Lambda task/indexer every day.
config3.json contains "archived" repositories (ones
which are not getting any more code updates). This one is not run
via a daily Lambda task, and need to be triggered manually if an
update is desired (generally not, since the code isn't changing).
Updates for this config should only be needed if the generated HTML
changes significantly.

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
in, you need to be a member of the
[searchfox-aws](https://people.mozilla.org/a/searchfox-aws/)
Mozillians access group.

### Fast way

The following is a perma-link that the slow way will also get you:
https://mozilla-aws.awsapps.com/start/#/saml/custom/653057761566%20%28Searchfox%20%28Mozilla%29%29/MzI5NTY3MTc5NDM2X2lucy0xMjRlYmMyMGY2ZGZhOTBkX3AtYTlkNGEyNTlkMjNhMjRkYw%3D%3D

### Slow way

Once you are a member, you can use your Mozilla SSO authentication to
log in to AWS by going to https://mozilla-aws.awsapps.com/start. Once you get past the
SSO authentication, you'll be asked to pick a role - the "AdministratorAccess"
role is almost always the one you will want, as it gives you access to make
changes whereas the other ones are read-only type roles.  Note that this URL
has changed with the retirement of the "maws" infrastructure.

### Once you're logged in.

After you've logged in, you need to [change the AWS region in the top right
corner](http://docs.aws.amazon.com/awsconsolehelpdocs/latest/gsg/getting-started.html#select-region). The
region for Searchfox is "US West (Oregon)". Now you should be able to
select EC2 from Services and see the list of EC2 machines running. The
tags on the machine can be useful in selecting a particular instance that
you might be looking for.

## Setting up AWS locally

Mozsearch uses a lot of scripts that use the AWS API to start and stop
indexing, provision servers, etc. It is recommended that you run these
scripts **outside** the VM, as that is where the commands below have been tested
and the SSO flows involve opening a web browser which is at best awkward from
inside the VM/docker instance.

We previously used `maws`, but consistent with the docs at
https://mozilla-hub.atlassian.net/l/cp/GH1kL2zg this mechanism has been retired
due to changes at our SSO provider.  In these docs, searchfox always falls under
"Mozilla IT".  If you have an existing maws setup and want to know how to
upgrade it, see the next markdown header section.

The first step is to install the AWS CLI v2.  Amazon intentionally does not
make this available as a package we can install via PIP, although there is a
third-party packaging available as a python package, that seems a bit like a
scary avenue for compromise, so I'm going to suggest:
- Linux:
  - Ubuntu:
    - &gt;= 24.04: The "awscli" package is gone and you have to install a snap:
      `sudo snap install aws-cli --classic`
    - &gt;= 23.04: The "awscli" package is v2 and can be installed via apt.
  - Debian: Yeah, v2 is available on some versions, `apt show awscli` and see if
    the major version is >= 2.  Keep upgrading until that's true.
  - Nix: the devShell in our flake.nix provides awscli2. Just call `nix develop`.
- OS X:
  - In the #iam channel, using "brew" was recommended.
- Everything else, or if the above didn't work for you:
  - https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html

Now when you run `aws --version` you should see something like (this is what I
see on Ubuntu 23.04):
```
$ aws --version
aws-cli/2.9.19 Python/3.11.4 Linux/6.2.0-32-generic source/x86_64.ubuntu.23 prompt/off
```

Now we are going to login to SSO.  We are going to run `aws configure sso` and
enter the following values.  Note that there's also a variation of this command,
`aws configure sso-session` and my eyes glazed over when trying to understand
the differences.  If you care, the docs are at
https://docs.aws.amazon.com/cli/latest/userguide/sso-configure-profile-token.html
and https://mozilla-hub.atlassian.net/l/cp/GH1kL2zg also I think tries to explain
the difference.

- For `SSO session name (Recommended):` enter `mozilla`
- For `SSO start URL [None]:` enter `https://mozilla-aws.awsapps.com/start`
- For `SSO region [None]:` enter `us-west-2`
- This will now attempt to open a web page and show a code that you should
  confirm is also present in the browser window.  (This ensures you're granting
  access to the CLI and not some other simultaneously popped up rogue auth
  request.)  Verify the code and press the button on the web page to advance.
- This will now ask "Allow botocore-client-mozilla to access your data?" and
  you should hit the allow button.
- You should now be done with the webpage, moving back to the CLI:
- It should indicate the AWS account id for searchfox is available to you,
  probably only that one.  If you're not sure which one to use (I'm avoiding
  listing the account id here), do the "Logging into the AWS console" section
  guidance above which will show you a list of account IDs and their actual
  account names.
- It will ask you which role you want to use.  Pick "AdministratorAccess".
- For `CLI default client Region [None]:` enter `us-west-2` again.
- For `CLI default output format [None]:` I just hit enter.
- For `CLI profile name [AdministratorAccess-###]:` I suggest using `searchfox`.

The CLI should then print something like:
```
To use this profile, specify the profile name using --profile, as shown:

aws s3 ls --profile searchfox
```

Run that command, and we should see something like the following:
```
$ aws s3 ls --profile searchfox
2020-06-27 09:19:43 indexer-logs
2022-08-06 13:48:44 searchfox-web-logs
2020-06-27 19:22:01 searchfox.repositories
```

Because we need the boto3 lib and a few other things, we're going to set up a
python venv now.  See https://packaging.python.org/en/latest/guides/installing-using-pip-and-virtual-environments/
for more on installing venv, but on ubuntu you should be able to run
`apt install python3-venv`.

> Note for Nix users: you don't need to setup a venv, the right Python packages
> are provided by our devShell.

```
# RUN THESE COMMANDS OUTSIDE THE VM!

python3 -m venv env
# if you chose a different profile name above, use that instead!
echo "export AWS_PROFILE=searchfox" >> env/bin/activate
# "." is the same as "source"
. env/bin/activate
pip install boto3 rich
# Make sure that we have an up-to-date version of certifi for certificate
# validation.  See comments in build-lambda-indexer-start.sh for more context.
pip install --upgrade certifi
```

Okay, now we're going to just validate that SSO credential refresh mechanism
works.  Run `aws sso login` and repeat the prompt flow where you have to:
- On the first page, verify that the code in the browser matches the code in the
  CLI and allow it.
- Allow "botocore-client-mozilla" access again.

Now we can run `infrastructure/aws/channel-tool.py list` or any other command.
But please run that one to make sure you see a list of our nodes, etc.

## Upgrading an existing maws configuration

The easiest option is to just `rm -rf env` the venv you previously created and
start over, especially if you just upgraded your distribution and pip may be in
a weird state.

That said, it's also possible you may have some weird system cruft built-up that
may need addressing, especially if you ever set things up before we started to
use venvs.  In that case you may do something like run "aws" and see something
like `-bash: /PATH/TO/.local/bin/aws: cannot execute: required file not found`.
In that case a fun thing to do is `rm ~/.local/bin/aws*`.  On bash, you will
likely then need to invoke `hash -r` so bash realizes those files have been
removed.

If you are deeply attached to your current venv, then you can:

```
# use the venv; we'll assume this is active from here on out
. env/bin/activate

# uninstall the old maws mechanism, you'll need to say yes.
pip uninstall mozilla-aws-cli-mozilla

# uninstall v1 awscli (binary "aws"), you'll need to say yes
pip uninstall awscli
```

Now go back to the previous section and install awscli.

## SSHing into AWS machines

To SSH into an EC2 instance, you will need to obtain the private key
file for Searchfox. Once you have the key, ensure that the permissions
are set so that it is not world-readable. Put it (or a symlink to it) at
`~/.aws/private_key.pem`. The ssh script below will check this location
and use this as the identity file if it exists.

Now you can connect to an instance as follows:

```
infrastructure/aws/ssh.py
```

This command will print a list of instances that you can connect to as
well as details about them. Select an instance ID (starting with `i-`)
and connect to it:

```
infrastructure/aws/ssh.py i-955af89
```

Passing a substring-match pattern as the 1st argument prints a list of
instances and the prompt to select the instance:

```
infrastructure/aws/ssh.py config1
infrastructure/aws/ssh.py web-server
```

Passing 2nd+ arguments executes the command in all matching instances.
The following command prints the result of `df -h` on all web-server
instances.

```
infrastructure/aws/ssh.py web-server df -h
```

## Lambda

The AWS Lambda task uses a cron-style scheduler to run once a day for all
configN jobs except config3 which runs once a week on Saturdays (which is the
day our scripts run maintenance, so it's the right day to pick when doing things
less than daily).  config3 previously ran only when manually triggered but this
resulted in both configuration bit-rot as well a tendency for config3 to be out
of date with regard to the other indexers which cause actual visible problems
when static resources changed.

### Automated-ish Updates

If you just want to update the existing daily lambda jobs for the release jobs,
you can:

- Inside the vagrant VM:
  - `cd /vagrant`
  - `./infrastructure/aws/build-lambda-zips-from-inside-vm.sh`
    - This will produce 6 zips files in `/vagrant` matching `lambda-release*.zip`
- Outside the vagrant VM where you have active credentials so that `ssh.py`
  works.  This will upload the zips and delete them after each successful upload
  so there should be no zip files left over.
  - `./infrastructure/aws/upload-lambda-zips-from-outside-vm.sh`

### Lambda Details / Manual Updates

The task that runs can be generated by running the following command inside
the Vagrant VM instance:

```
./infrastructure/aws/build-lambda-indexer-start.sh \
  https://github.com/mozsearch/mozsearch \
  https://github.com/mozsearch/mozsearch-mozilla \
  config1.json \
  master \
  release1
```

The first three arguments are links to the repositories containing
Mozsearch and the configuration to use. The fourth argument is a branch
name. When scripts check out Mozsearch or the configuration
repository, they will check out this branch. The last argument is used
to determine which ELB target group will be updated. The `release1`
argument updates the `release1-target` target group (which might
control, for example, `example.com`). The `dev` argument updates the
`dev-target` target group (which might control, for example,
`dev.example.com`).

When the script finishes, it generates a file, `/tmp/lambda.zip`, that
can be uploaded to AWS Lambda using the AWS control panel. To update
an existing lambda task, select that task from the AWS Lambda console,
scroll down to the "Function code" section, and select "Upload a .zip file"
from the Actions menu. Save your changes and that should be all that
you need.

If you're setting up a new Lambda task for a new channel:
- Select "Create Function" from the AWS Lambda console.
  - Give it a name similar to the others (`start-<channel>-indexer`),
  - Select Python 3.12 for the Runtime
  - Expand "Change default execution role" and the "Use an existing role" radio
    button, then select the existing `lambda_indexer_start_role` for the
    execution role. This gives the task permissions to create indexer instances.
  - Press the "Create function" button to create the rule.
- On the function's page (you may have to click on the newly created rule or you
  may end up there automatically):
  - On the "Code" tab which you should already be looking at:
    - Click "Edit" by runtime settings and set the Handler to
     `lambda-indexer-start.start` (this refers to the `start` function in the
     `lambda-indexer-start.py` file inside the generated `lambda.zip`).
    - Hit "Save" to get back to the code tab.
  - On the "Configuration" time tab (which you will need to click on):
    - On the "General configuration" vertical sub-tab:
      - Click the "Edit" button for "general configuration" section so we can
        edit the "Timeout" setting.
      - The default timeout is 3 seconds which is way too short.  Increase that
        to 1 minute.
  - At the top of the UI in the "Function Overview" box you can add a cron
    trigger by pressing the "Add trigger" button and selecting "EventBridge
    (CloudWatch Events)" as a source from the drop-down and select one of the
    existing "everyday" rules or create a new one as appropriate.

## Triggering indexing manually

### Via the AWS UI

From the Lambda UI's list of functions you can:
- Select the function you want to run by clicking on it and bringing up its UI.
- Click on the "Test" tab.
- Click on the "Test Event" section's "Test" button.  If you wait about 12
  seconds an AJAX UI should do a spinny thing and then show a green
  "Execution result: succeeded".  If you see a failure and if it mentions a
  timeout, then that means you probably forgot to increase the timeout from the
  default of 3 seconds when creating the function.  Update it now!

### Locally

It's fairly easy to trigger an indexing job manually from your local
computer. To do so, run the following from within the Python virtual environment:

```
infrastructure/aws/trigger_indexer.py \
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

## Creating additional development channels

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

## Creating additional release channels

If more release channels are required (usually because we want to
host even more repos and the existing indexers/web-servers are
nearing their capacity limits), the process is a little different
than that for creating additional development channels as described
above. There is only one load balancer for all release channels,
so you don't have to create one. However, you do need to create a
new target group. Make sure it starts with the string "release" as
this is handled specially within parts of the Mozsearch codebase.

Once you've created a new target group, you can kick off an indexer
and/or set up a lambda task for this channel using your desired
config file. The only other step required is to modify the `release-lb`
load balancer to direct requests for those new repos to the appropriate
target group. Do this by selecting the `release-lb` load balancer in
the AWS EC2 console, going to the listeners tab, and editing the rules.
Note that you need to edit the rules for both HTTP and HTTPS manually.
The rule editor is fairly self-explanatory, just add new rules
in the (ordered) list to redirect requests for the new repos to the
new target group.

## Provisioning and cloud init

The EC2 instances for indexing and web serving are started using a
custom Amazon Machine Image (AMI). This is the disk image used for
booting the machine. These AMIs are based off Ubuntu 20.04, but
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
version of Clang is required, for example). We've also recently started
re-provisioning whenever we update Cargo.toml dependencies so that the
update process is less likely to fail due to download failures (which
was happening frequently enough that we started doing this.)

Dependencies that aren't handled by the build system need to be expressed in
our shell scripts:
- infrastructure/aws/indexer-provision.sh: AWS-specific dependencies/setup for
  the indexing process.  This runs before the normal indexer provisioning script
  in order to perform setup like resizing the EBS boot partition.
- infrastructure/indexer-provision.sh: Dependencies for indexing both in the
  local dev VM and on an AWS instance.
- infrastructure/aws/web-server-provision.sh: AWS-specific dependencies/setup
  for the indexing process.  This will tend to be a subset of the indexer setup
  because there's less to run on the web-server and we also don't give the web
  server an IAM role so it can't do as much infrastructure manipulation.  (These
  things must be done on behalf of the web-server by the indexer that is
  starting the web-server.)
- infrastructure/web-server-provision.sh: Dependences for web-serving in the
  local dev VM and on an AWS instance.  Because the dev VM will also run the
  indexer provisioning scripts, this script should ideally avoid doing redundant
  work.  However, it's not required for this script to succeed if it's run a
  second time itself; we no longer support re-provisioning the dev VM manually.
  (Instead, the VM should be destroyed and rebuilt.)

Generating a new AMI should now be largely automated thanks to the work on
[bug 1747289](https://bugzilla.mozilla.org/show_bug.cgi?id=1747289).
However, there are a set of manual steps that need to be taken, see below.

To re-provision the indexer AMI, run the following:

```
infrastructure/aws/trigger-provision.py indexer \
  infrastructure/aws/indexer-provision.sh \
  infrastructure/common-provision-pre.sh \
  infrastructure/indexer-provision.sh \
  infrastructure/common-provision-post.sh
```

For web serving, use this command:

```
infrastructure/aws/trigger-provision.py web-server \
  infrastructure/aws/web-server-provision.sh \
  infrastructure/common-provision-pre.sh \
  infrastructure/web-server-provision.sh \
  infrastructure/common-provision-post.sh
```

The `trigger-provision.py` script starts a new EC2 instance and uses
cloud-init to run the given provisioner shell scripts in it. These
scripts:
- Install all the required dependencies.
- Create a new AMI image named `{indexer/web-server}-YEAR-MONTH-DAY-HOUR-MINUTE`
  (well, that's the template).
- Wait for the image to be created; an S3 snapshot needs to be performed and
  this takes on the order of 10 minutes.
- Tag the new image with "indexer" or "web-server" as appropriate.
- remove the tag from the old image.
- send an email about success/failure
  - Disclaimer: Depending on when provisioning fails, it's possible that the
    system state will mean that it's not possible for a failure email to be
    sent.

In the event of failure, the EC2 instance will shut itself down via `shutdown`
with a 10 minute delay which means that you can inspect the failure by canceling
shutdown with `sudo shutdown -c` if you log in before shutdown, or by restarting
the instance if the instance has already shut down.  The `ssh.py` command will
offer to start the instance if it's stopped, so no extra steps are required.

### Still-Required Manual Steps

The following will continue to need to be done eventually, at least until
more automation is put in place.
1. The old AMIs need to be deleted.  Each AMI uses S3 storage and has an
   associated (low) cost, and we don't really need more than one backup or even
   a backup after a successful indexing run, so it's likely best to delete the
   old AMIs a few days after provisioning.
   - Deregistering is accomplished by:
     - Going to the EC2 console and clicking on "AMIs" under the "Images"
       heading to get a list of current AMIs.
     - Click on the AMI you think you want to delete.  Because of the date-based
       naming scheme, this should be an AMI with an older name.
     - Confirm that the AMI is not currently tagged for use.  Specifically,
       there should be no tags listed, resulting in "No tags found" being
       displayed.
     - Click the "Actions" button up at the top of the pane and select
       "Deregister AMI".
   - You shouldn't need to worry about any side-effects on existing EC2
     instances because they effectively fork a copy-on-write version of the AMI
     at startup.
2. The volume snapshots corresponding to the old AMIs need to be deleted. As
   with the AMI, the volume snapshot uses has an ongoing cost.
   AWS automatically prevents you from deleting snapshots that are associated
   with a still-active AMI, so the easiest way to purge unused snapshots is to:
   - Go to the snapshots pane (under "Elastic Block Store" heading in the EC2
     console.
   - Select all the snapshots
   - From the actions menu, select delete.
   - Confirm the deletion as requested
   - This will fail to delete some snapshots (because they are currently in
     use by some AMI) and delete all the unused ones.
   - Verify that the number of snapshots remaining is equal to the number of
     AMIs (as of this writing at least, each AMI generates one volume snapshot).

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
sudo mount /dev/`lsblk | grep 300G | cut -d" " -f1` /index
```
to re-mount the data volume. This will allow you to inspect the state
on the data volume as well as run additional commands for debugging
purposes, or to test a fix.  Note that for release2 "400G" should be used
instead of "300G".

Because the AWS indexing jobs now use a scratch-disk and that's relevant
for the indexing process, when the indexer aborts, it moves the contents
of `/index` under an `interrupted` directory on the above
mount point.  So the in-progress indexing data can be found at
`/index-ebs/interrupted` after the above mount.  In order to make paths sane
again, you can run the command:
```
sudo ln -s /index-ebs/interrupted /index
```
to provide the same effective path mapping.  Note that you wouldn't want
to restart indexing under this regime as `/index-ebs` would
be backed by IO-bound S3. If you *do* want to do I/O intensive work
in this state, you can move the interrupted state back to a local
disk by running:
```
$HOME/mozsearch/infrastructure/aws/mkscratch.sh
mv /index-ebs/interrupted/* /index/ # may take a long time
```

The shell scripts that run during indexing
generally require some environment variables to be set; you can set
up the main ones by sourcing the load-vars.sh script like so:
```
export MOZSEARCH_PATH=$HOME/mozsearch
# Replace the last two arguments with the appropriate config file
# and repo that errored out
source $MOZSEARCH_PATH/scripts/load-vars.sh $HOME/config/config.json firefox-main
```

After the debugging is complete, or even if no SSHing is required,
it is important to terminate the indexer and delete the incomplete
index volume, otherwise they will sit around forever and eat up money.
You can terminate the indexer either through the EC2 web console, or
by running
```
infrastructure/aws/terminate-indexer.py <instance-id>
infrastructure/aws/delete-volume.py <volume-id>
```
from within your local searchfox venv (see the above section
on setting up AWS locally). The terminate-indexer.py script or the
web console will let you know the volume ID of the volume to delete.

## Migrate the web-server for emergency reasons

In case the web-server instance bumps into troubles, such as it becomes
unresponsive, you can migrate to a new web-server instance reusing the
result of index by the following command, replacing each uppercase arguments:

```
infrastructure/aws/trigger-web-server.py \
  CHANNEL \
  https://github.com/SOME_USER/mozsearch \
  MOZSEARCH_REV \
  https://github.com/SOME_USER/mozsearch-mozilla \
  CONFIG_REV \
  CONFIG_FILE.JSON \
  INDEX_VOLUME_ID \
  "-" \
  LOCAL_CONFIG_REPO_PATH \
  "-" \
  BRANCH_NAME_FOR_DISPLAY
```

This script performs the same thing as the indexer which starts a new
web-server instance, except for the following:
  * Reuse the `/index` volume in `INDEX_VOLUME_ID` (`vol-***`)
  * Forcibly detach the volume from the already-attached instances
  * Do not perform automatic check for the server statup.  Please follow the message and manually perform it
