MOUNT_POINT = '/home/vagrant/mozsearch'

Vagrant.configure("2") do |config|
    config.vm.box_url = "http://cloud-images.ubuntu.com/vagrant/trusty/current/trusty-server-cloudimg-amd64-vagrant-disk1.box"
    config.vm.box = "ubuntu/trusty64"

    config.vm.provider "virtualbox" do |v|
        v.name = "MOZSEARCH_VM"
        v.customize ["setextradata", :id,
            "VBoxInternal2/SharedFoldersEnableSymlinksCreate//home/vagrant/mozsearch", "1"]
    end

    config.vm.synced_folder ".", MOUNT_POINT

    config.vm.provision "shell", path: "vagrant_provision.sh"

    config.vm.network "forwarded_port", guest: 8000, host: 8000
end
