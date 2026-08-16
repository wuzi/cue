project(
  'cue',
  'rust',
  version: '0.2.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.3.0',
)

gnome = import('gnome')
i18n = import('i18n')
cargo = find_program('cargo')
shell = find_program('sh')

prefix = get_option('prefix')
bindir = get_option('bindir')
datadir = get_option('datadir')
localedir = get_option('localedir')
app_id = 'io.github.wuzi.Cue'

custom_target(
  'cue-binary',
  output: 'cue',
  build_always_stale: true,
  command: [
    shell,
    files('build-aux/cargo-build.sh'),
    meson.project_source_root(),
    '@OUTPUT@',
    meson.project_build_root() / 'cargo-target',
    prefix / localedir,
  ],
  build_by_default: true,
  install: true,
  install_dir: bindir,
)

install_data(
  'data' / app_id + '.desktop',
  install_dir: datadir / 'applications',
)
install_data(
  'data' / app_id + '.metainfo.xml',
  install_dir: datadir / 'metainfo',
)
foreach icon_size : ['128x128', '256x256', '512x512']
  install_data(
    'data/icons/hicolor' / icon_size / 'apps' / app_id + '.png',
    install_dir: datadir / 'icons/hicolor' / icon_size / 'apps',
  )
endforeach
install_data(
  'data/icons/hicolor/symbolic/apps' / app_id + '-symbolic.svg',
  install_dir: datadir / 'icons/hicolor/symbolic/apps',
)

service_configuration = configuration_data()
service_configuration.set('bindir', prefix / bindir)
configure_file(
  input: 'data' / app_id + '.service.in',
  output: app_id + '.service',
  configuration: service_configuration,
  install: true,
  install_dir: datadir / 'dbus-1/services',
)

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)

subdir('po')
