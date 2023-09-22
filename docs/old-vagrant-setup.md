## Deprecated Vagrant Setup

There are almost no circumstances in which you'd want to use this Vagrant setup,
and with any luck it will bit-rot into obsolescence and we can delete this doc
and the Vagrant file, but... here you go:

### Setting up the VM

We use Vagrant to setup a virtual machine.  This may be the most frustrating part of
working with Searchfox.  If you can help provide better/more explicit instructions
for your platform, please do!

#### Linux

Important note: In order to expose the Searchfox source directory into the VM, we
need to be able to export it via NFS.  If you are using a FUSE-style filesystem
like `eCryptFS` which is a means of encrypting your home directory, things will not
work.  You will need to move searchfox to a partition that's a normal block device
(which includes LUKS-style encrypted partitions, etc.)

##### Ubuntu

```shell
# make sure the apt package database is up-to-date
sudo apt update
# vagrant will also install vagrant-libvirt which is the vagrant provider we use.
# virt-manager is a UI that helps inspect that your VM got created
# The rest are related to enabling libvirt and KVM-based virtualization
sudo apt install vagrant virt-manager qemu libvirt-daemon-system libvirt-clients

git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init
vagrant up
```
##### Other Linux
Note: VirtualBox is an option on linux, but not recommended.

1. [install Vagrant](https://www.vagrantup.com/downloads.html).
2. Install libvirt via [vagrant-libvirt](https://github.com/vagrant-libvirt/vagrant-libvirt).
   Follow the [installation instructions](https://github.com/vagrant-libvirt/vagrant-libvirt#installation).
  - Note that if you didn't already have libvirt installed, then a new `libvirt`
    group may just have been created and your existing logins won't have the
    permissions necessary to talk to the management socket.  If you do
    `exec su -l $USER` you can get access to your newly assigned group.
  - See troubleshooting below if you have problems.

Once that's installed:
```shell
git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init
vagrant up
```

If vagrant up times out in the "Mounting NFS shared folders..." step, chances
are that you cannot access nfs from the virtual machine.

Under stock Fedora 31, you probably need to allow libvirt to access nfs:

```
firewall-cmd --permanent --add-service=nfs --zone=libvirt
firewall-cmd --permanent --add-service=rpc-bind --zone=libvirt
firewall-cmd --permanent --add-service=mountd --zone=libvirt
firewall-cmd --reload
```

#### OS X and Windows

Note: The current Homebrew version of Vagrant is currently not able to use the most
recent version of VirtualBox so it's recommended to install things directly via their
installers.

1. [install Vagrant](https://www.vagrantup.com/downloads.html).
2. Figure out the right virtualization option for you.
  - OS X:
    - Are you on an M1 mac?  Then you probably need to get a license for Parallels
      and use it, maybe.  And then you can do `vagrant plugin install vagrant-parallels`
      below.
    - Maybe get a license for parallels anyways?
    - Otherwise do the virtualbox thing below.
  - Windows,  Visit the [VirtualBox downloads page](https://www.virtualbox.org/wiki/Downloads) and
    follow the instructions for your OS.  You do not need and should not install
    any extra extensions.  You only need the Open Source piece and should avoid
    installing anything closed source or with a commercial license.

Then clone Mozsearch and provision a Vagrant instance:
```
git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init

# If using VirtualBox; if using Parallels, install `vagrant-parallels`
vagrant plugin install vagrant-vbguest
vagrant up
```

### Once vagrant up has started...

The last step will take some time (10 or 15 minutes on a fast laptop)
to download a lot of dependencies and build some tools locally.  **Note
that this step can fail!**  Say, if you're at a Mozilla All-Hands and the
network isn't exceedingly reliable.  In particular, if you are seeing
errors related to host resolution and you have access to a VPN, it may
be advisable to connect to the VPN.

A successful provisioning run will end with `mv update-log provision-update-log-2`.

In the event of failure you will want to run
`vagrant destroy` to completely delete the VM and then
run `vagrant up` again to re-create it.  The base image gets cached on
your system, so you'll save ~1GB of download, but all the Ubuntu package
installation will be re-done.

After `vagrant up` completes, ssh into the VM as follows. From this point
onward, all commands should be executed inside the VM.

```
vagrant ssh
```

At this point, your Mozsearch git directory has been mounted into a
shared folder at `/vagrant` in the VM. Any changes made from inside or
outside the VM will be mirrored to the other side. Generally I find it
best to edit code outside the VM, but any commands to build or run
scripts must run inside the VM.
