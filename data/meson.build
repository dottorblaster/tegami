desktop_conf = configuration_data()
desktop_conf.set('app-id', app_id)

desktop_file = configure_file(
  input: '@0@.desktop.in'.format(app_id),
  output: '@0@.desktop'.format(app_id),
  configuration: desktop_conf,
  install: true,
  install_dir: datadir / 'applications',
)

if desktop_file_validate.found()
  test('validate-desktop', desktop_file_validate, args: [desktop_file.full_path()])
endif

metainfo_conf = configuration_data()
metainfo_conf.set('app-id', app_id)

metainfo_file = configure_file(
  input: '@0@.metainfo.xml.in'.format(app_id),
  output: '@0@.metainfo.xml'.format(app_id),
  configuration: metainfo_conf,
  install: true,
  install_dir: datadir / 'metainfo',
)

if appstream_cli.found()
  test('validate-metainfo', appstream_cli, args: ['validate', '--no-net', metainfo_file.full_path()])
elif appstream_util.found()
  test('validate-metainfo', appstream_util, args: ['validate', '--nonet', metainfo_file.full_path()])
endif

configure_file(
  input: '@0@.gschema.xml'.format(app_id),
  output: '@0@.gschema.xml'.format(app_id),
  copy: true,
  install: true,
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

test('validate-gschema', glib_compile_schemas, args: ['--strict', '--dry-run', meson.current_build_dir()])

data_resources = gnome.compile_resources(
  'tegami-data',
  'tegami.gresource.xml',
  source_dir: meson.current_source_dir(),
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
)

subdir('icons')