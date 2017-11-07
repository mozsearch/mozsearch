Vagrant.configure("2") do |config|
  # We use this image to get a large (160GB) disk rather than the normal 40GB.
  config.vm.box = "cbednarski/ubuntu-1604-large"
  config.vm.box_version = "0.1.0"

  config.vm.provision :shell, path: "infrastructure/indexer-provision.sh"
  config.vm.provision :shell, path: "infrastructure/vagrant/indexer-provision.sh"
  config.vm.provision :shell, path: "infrastructure/web-server-provision.sh"

  config.vm.network :forwarded_port, guest: 80, host: 8000

  config.vm.provider "virtualbox" do |v|
    host = RbConfig::CONFIG['host_os']
    if host =~ /darwin/
      # sysctl returns Bytes and we need to convert to MB
      mem = `sysctl -n hw.memsize`.to_i / 1024
    elsif host =~ /linux/
      # meminfo shows KB and we need to convert to MB
      mem = `grep 'MemTotal' /proc/meminfo | sed -e 's/MemTotal://' -e 's/ kB//'`.to_i
    elsif host =~ /mswin|mingw|cygwin/
      # Windows code via https://github.com/rdsubhas/vagrant-faster
      mem = `wmic computersystem Get TotalPhysicalMemory`.split[1].to_i / 1024
    end

    # Give VM 1/2 system memory
    v.memory = mem / 1024 / 2
    v.cpus = 4
  end
end
