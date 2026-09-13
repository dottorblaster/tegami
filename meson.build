project(
  'tegami',
  'rust',
  version: '0.1.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.4.0',
)

gnome = import('gnome')

app_id = 'it.dottorblaster.tegami'

cargo = find_program('cargo', required: true)
blueprint_compiler = find_program('blueprint-compiler', required: true)
glib_compile_schemas = find_program('glib-compile-schemas', required: true)
desktop_file_validate = find_program('desktop-file-validate', required: false)
appstream_cli = find_program('appstreamcli', required: false)
appstream_util = find_program('appstream-util', required: false)

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
pkgdatadir = datadir / meson.project_name()
iconsdir = datadir / 'icons' / 'hicolor'

subdir('data')

cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', meson.project_build_root() / 'target',
]

if get_option('buildtype') in ['release', 'debugoptimized']
  cargo_options += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: bindir,
  depends: data_resources,
  command: [
    'env',
    'TEGAMI_APP_ID=@0@'.format(app_id),
    'TEGAMI_VERSION=@0@'.format(meson.project_version()),
    'TEGAMI_RESOURCE_BUNDLE=@0@'.format(data_resources.full_path()),
    cargo, 'build', cargo_options,
    '&&',
    'cp',
    meson.project_build_root() / 'target' / rust_target / meson.project_name(),
    '@OUTPUT@',
  ],
)

gnome.post_install(glib_compile_schemas: true)