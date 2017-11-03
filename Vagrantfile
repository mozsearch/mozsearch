Vagrant.configure("2") do |config|
  config.vm.box = "bento/ubuntu-16.04"

  config.vm.provision :shell, path: "infrastructure/indexer-provision.sh"
  config.vm.provision :shell, path: "infrastructure/vagrant/indexer-provision.sh"
  config.vm.provision :shell, path: "infrastructure/web-server-provision.sh"

  config.vm.network :forwarded_port, guest: 80, host: 8000

  config.vm.provider "virtualbox" do |v|
    v.memory = 32768
    v.cpus = 4
  end
end
