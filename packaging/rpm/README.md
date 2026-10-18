# Building the RPM on Fedora

NOT TESTED. Written without access to a Fedora system; the script flow was dry-run with
stand-in tools only. Compile errors, missing packages or RPM mistakes may still appear.

1. Install tools:
   sudo dnf install rpm-build gcc gcc-c++ make pkgconf-pkg-config glib2-devel \
        gstreamer1-devel gstreamer1-plugins-base-devel python3
   (Optional, for automatic game capture hooks:
    sudo dnf install glibc-devel.i686 libstdc++-devel.i686 vulkan-headers libglvnd-devel)
2. Install Rust 1.90+ with rustup (https://rustup.rs), then reopen the terminal.
3. From the source folder: packaging/rpm/build-rpm.sh
4. Install: sudo dnf install ./dist/rosorah-studio-*.rpm
5. Start from the application menu, or run: rosorah-studio

Installed layout: /opt/rosorah-studio (program, browser helper, optional hooks),
/usr/bin/rosorah-studio (launcher), /usr/share/applications/io.rosorah.Studio.desktop.
Project files and settings stay in your home folder and are kept on upgrade or removal.
Remove with: sudo dnf remove rosorah-studio
The license field in rosorah-studio.spec is a placeholder; edit it before distributing.
