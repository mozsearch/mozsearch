Vagrant.configure("2") do |config|
  config.vm.box = "bento/ubuntu-16.04"

  config.vm.define "indexer" do |indexer|
    indexer.vm.provision "install", type:"shell", path: "infrastructure/indexer-provision.sh"
    indexer.vm.provision "build", type:"shell", path: "infrastructure/vagrant/indexer-provision.sh"

    indexer.vm.provider "virtualbox" do |v|
      v.memory = 32768
      v.cpus = 4
    end
  end

  config.vm.define "web" do |web|
    web.vm.provision :shell, path: "infrastructure/web-server-provision.sh"
    web.vm.network :forwarded_port, guest: 80, host: 8000
  end
end
