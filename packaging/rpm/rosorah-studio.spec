# Packages an already-built Rosorah Studio. Run build-rpm.sh; it compiles the app,
# stages the files and calls rpmbuild with the values defined below.
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
# The optional 32-bit game-capture hooks must not force 32-bit system libraries to be installed.
%global __requires_exclude_from ^/opt/rosorah-studio/hooks/.*$
%global __provides_exclude_from ^/opt/rosorah-studio/hooks/.*$

Name:           rosorah-studio
Version:        %{pkgversion}
Release:        1%{?dist}
Summary:        Streaming, recording and multistreaming studio
License:        Proprietary
ExclusiveArch:  x86_64

Requires:       gstreamer1
Requires:       gstreamer1-plugins-base
# FFmpeg-based decoders for alert media: animated GIF, AAC/M4A and other formats.
Requires:       gstreamer1-plugin-libav
Requires:       pipewire-gstreamer
Requires:       pipewire-utils
Requires:       python3
# ffmpeg-free (Fedora) or ffmpeg (RPM Fusion) both provide this.
Requires:       /usr/bin/ffmpeg

%description
Rosorah Studio is a streaming and recording application with scenes, two canvases,
multistreaming, unified chat, alerts and automations. This is a locally built,
unsigned development package.

%prep
# Nothing to unpack; the build script supplies %{stagedir}.

%build
# Compiled by build-rpm.sh before rpmbuild runs.

%install
install -d %{buildroot}/opt/rosorah-studio %{buildroot}%{_bindir} %{buildroot}%{_datadir}/applications %{buildroot}%{_datadir}/icons/hicolor/scalable/apps
cp -a %{stagedir}/. %{buildroot}/opt/rosorah-studio/
install -m 0755 %{stagedir}/../rosorah-studio-wrapper %{buildroot}%{_bindir}/rosorah-studio
install -m 0644 %{stagedir}/../io.rosorah.Studio.desktop %{buildroot}%{_datadir}/applications/io.rosorah.Studio.desktop
install -m 0644 %{stagedir}/../io.rosorah.Studio.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.rosorah.Studio.svg

%files
/opt/rosorah-studio
%{_bindir}/rosorah-studio
%{_datadir}/applications/io.rosorah.Studio.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.rosorah.Studio.svg

%changelog
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-12
- Stinger video and transition screen scene transitions.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-11
- Now playing: font and size for artist and album; optional album line.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-10
- Require gstreamer1-plugin-libav for wider alert media format support.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-9
- Friendlier alerts and automations; built-in alert sounds and animations; light mode, backgrounds and glass panels.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-8
- TikTok LIVE chat, gifts, follows and subscriptions from the LIVE page; opt-in TikTok sending.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-7
- Connections page, layer Properties window, simpler Add layer menu, proportional edge resizing.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-6
- Chat moderation menus for Twitch, YouTube and Kick; per-platform clear options.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-5
- Keep the Kick chat page laid out while the chat browser runs in the background.
* Thu Oct 15 2026 Local build <local@localhost> - %{pkgversion}-4
- Read chat from the current Kick chat page layout.
* Wed Oct 14 2026 Local build <local@localhost> - %{pkgversion}-3
- Chat list fills the panel width; clearer Kick error messages.
* Wed Oct 14 2026 Local build <local@localhost> - %{pkgversion}-2
- Add the app icon and "Send to all" chat label.
* Wed Oct 14 2026 Local build <local@localhost> - %{pkgversion}-1
- Local package of Rosorah Studio, including Kick chat and broadcast details.
