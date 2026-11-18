# Andromeda RPM. Binaries are built beforehand by scripts/build-rpm.sh
# (cargo + trunk need network access, which rpmbuild should not have);
# this spec only installs the staged files.

# Prebuilt binaries: no debuginfo package.
%global debug_package %{nil}
%global app_id io.github.incept0s.Andromeda
%global ext_uuid andromeda@incept0s.github.io

Name:           andromeda
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        System monitor, volume mixer and everyday tools for GNOME
License:        GPL-3.0-or-later
URL:            https://github.com/incept0s/andromeda
Source0:        %{name}-%{version}.tar.gz
ExclusiveArch:  x86_64

BuildRequires:  systemd-rpm-macros

Requires:       gnome-shell >= 50
Requires:       webkit2gtk4.1
Requires:       curl
# Media tools and the converter. A file dependency is satisfied by
# Fedora's ffmpeg-free and by RPM Fusion's ffmpeg alike.
Requires:       /usr/bin/ffmpeg
# Copy text from screen.
Requires:       tesseract
Requires:       tesseract-langpack-eng
# Optional: everything still works without them, the feature says what
# is missing instead.
Recommends:     pipewire-pulseaudio
Recommends:     libreoffice-writer
Recommends:     PackageKit
Recommends:     flatpak
Recommends:     udisks2

%description
Andromeda puts a system monitor, a per-app volume mixer, keep awake,
network and disk details, clipboard history, text recognition, a file
converter, a cleaner and package management behind one top-bar icon in
GNOME, with an Alt+Tab switcher that shows large window previews.

Inspired by Vorssaint Utils; not affiliated with or endorsed by Vorssaint.

%prep
%setup -q

%install
cp -a usr %{buildroot}/

%post
%systemd_user_post andromedad.service

%preun
%systemd_user_preun andromedad.service

%files
%license usr/share/licenses/%{name}/LICENSE
%doc usr/share/doc/%{name}/README.md
%doc usr/share/doc/%{name}/CHANGELOG.md
%doc usr/share/doc/%{name}/THIRD-PARTY.md
%doc usr/share/doc/%{name}/THIRD-PARTY-LICENSES.md
%{_bindir}/andromeda
%{_libexecdir}/andromedad
%{_userunitdir}/andromedad.service
%{_userpresetdir}/80-andromeda.preset
%{_datadir}/dbus-1/services/%{app_id}.service
%{_datadir}/dbus-1/services/%{app_id}.Panel.service
%{_datadir}/applications/%{app_id}.desktop
%{_datadir}/icons/hicolor/scalable/apps/%{app_id}.svg
%{_datadir}/icons/hicolor/symbolic/apps/%{app_id}-symbolic.svg
%{_datadir}/gnome-shell/extensions/%{ext_uuid}/

%changelog
* Tue Nov 17 2026 incept0s - 0.1.0-1
- First release
