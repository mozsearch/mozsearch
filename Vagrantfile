Vagrant.configure("2") do |config|
  config.vm.box = "ubuntu/trusty64"

  config.vm.define "indexer" do |indexer|
    indexer.vm.provision :shell, path: "infrastructure/indexer-provision.sh"

    indexer.vm.provider "virtualbox" do |v|
      v.memory = 8192
      v.cpus = 2
    end
  end

  config.vm.define "web" do |web|
    web.vm.provision :shell, path: "infrastructure/web-provision.sh"
    web.vm.network :forwarded_port, guest: 80, host: 8000
  end
end
