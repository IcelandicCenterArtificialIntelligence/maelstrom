#![allow(unused_imports)]
use super::container::{ContainerField, TestContainer, TestContainerVisitor};
use anyhow::Result;
use maelstrom_base::{GroupId, JobMountForTomlAndJson, JobNetwork, Timeout, UserId, Utf8PathBuf};
use maelstrom_client::spec::{incompatible, Image, ImageUse, LayerSpec, PossiblyImage};
use serde::{de, Deserialize, Deserializer};
use std::{
    collections::BTreeMap,
    fmt::Display,
    str::{self, FromStr},
};

#[derive(Debug, PartialEq)]
pub struct TestDirective<TestFilterT> {
    pub filter: Option<TestFilterT>,
    pub container: TestContainer,
    pub include_shared_libraries: Option<bool>,
    pub timeout: Option<Option<Timeout>>,
    pub ignore: Option<bool>,
}

// The derived Default will put a TestFilterT: Default bound on the implementation
impl<TestFilterT> Default for TestDirective<TestFilterT> {
    fn default() -> Self {
        Self {
            filter: None,
            container: Default::default(),
            include_shared_libraries: None,
            timeout: None,
            ignore: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum DirectiveField {
    Filter,
    IncludeSharedLibraries,
    Timeout,
    Ignore,
    Network,
    EnableWritableFileSystem,
    User,
    Group,
    Mounts,
    AddedMounts,
    Image,
    WorkingDirectory,
    Layers,
    AddedLayers,
    Environment,
    AddedEnvironment,
}

impl DirectiveField {
    fn into_container_field(self) -> Option<ContainerField> {
        match self {
            Self::Filter => None,
            Self::IncludeSharedLibraries => None,
            Self::Timeout => None,
            Self::Ignore => None,
            Self::Network => Some(ContainerField::Network),
            Self::EnableWritableFileSystem => Some(ContainerField::EnableWritableFileSystem),
            Self::User => Some(ContainerField::User),
            Self::Group => Some(ContainerField::Group),
            Self::Mounts => Some(ContainerField::Mounts),
            Self::AddedMounts => Some(ContainerField::AddedMounts),
            Self::Image => Some(ContainerField::Image),
            Self::WorkingDirectory => Some(ContainerField::WorkingDirectory),
            Self::Layers => Some(ContainerField::Layers),
            Self::AddedLayers => Some(ContainerField::AddedLayers),
            Self::Environment => Some(ContainerField::Environment),
            Self::AddedEnvironment => Some(ContainerField::AddedEnvironment),
        }
    }
}

struct DirectiveVisitor<TestFilterT> {
    value: TestDirective<TestFilterT>,
    container_visitor: TestContainerVisitor,
}

impl<TestFilterT> Default for DirectiveVisitor<TestFilterT> {
    fn default() -> Self {
        Self {
            value: Default::default(),
            container_visitor: Default::default(),
        }
    }
}

impl<TestFilterT: FromStr> DirectiveVisitor<TestFilterT>
where
    TestFilterT::Err: Display,
{
    fn fill_entry<'de, A>(&mut self, ident: DirectiveField, map: &mut A) -> Result<(), A::Error>
    where
        A: de::MapAccess<'de>,
    {
        match ident {
            DirectiveField::Filter => {
                self.value.filter = Some(
                    map.next_value::<String>()?
                        .parse()
                        .map_err(de::Error::custom)?,
                );
            }
            DirectiveField::IncludeSharedLibraries => {
                self.value.include_shared_libraries = Some(map.next_value()?);
            }
            DirectiveField::Timeout => {
                self.value.timeout = Some(Timeout::new(map.next_value()?));
            }
            DirectiveField::Ignore => {
                self.value.ignore = Some(map.next_value()?);
            }
            c => {
                self.container_visitor
                    .fill_entry(c.into_container_field().unwrap(), map)?;
            }
        }
        Ok(())
    }
}

impl<'de, TestFilterT: FromStr> de::Visitor<'de> for DirectiveVisitor<TestFilterT>
where
    TestFilterT::Err: Display,
{
    type Value = TestDirective<TestFilterT>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "TestDirective")
    }

    fn visit_map<A>(mut self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        while let Some(key) = map.next_key()? {
            self.fill_entry(key, &mut map)?;
        }

        self.value.container = self.container_visitor.into_test_container();
        Ok(self.value)
    }
}

impl<'de, TestFilterT: FromStr> de::Deserialize<'de> for TestDirective<TestFilterT>
where
    TestFilterT::Err: Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DirectiveVisitor::<TestFilterT>::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::container::NamedTestContainer;
    use anyhow::Error;
    use indoc::indoc;
    use maelstrom_base::{enum_set, JobDeviceForTomlAndJson};
    use maelstrom_client::spec::SymlinkSpec;
    use maelstrom_test::{
        glob_layer, non_root_utf8_path_buf, paths_layer, so_deps_layer, string, tar_layer,
        utf8_path_buf,
    };
    use toml::de::Error as TomlError;

    fn parse_test_directive(file: &str) -> Result<TestDirective<String>> {
        toml::from_str(file).map_err(Error::new)
    }

    fn parse_test_container(file: &str) -> Result<NamedTestContainer> {
        toml::from_str(file).map_err(Error::new)
    }

    fn assert_toml_error(err: Error, expected: &str) {
        let err = err.downcast_ref::<TomlError>().unwrap();
        let message = err.message();
        assert!(message.starts_with(expected), "message: {message}");
    }

    fn directive_error_test(toml: &str, error: &str) {
        assert_toml_error(parse_test_directive(toml).unwrap_err(), error);
    }

    fn directive_or_container_error_test(toml: &str, error: &str) {
        assert_toml_error(parse_test_directive(toml).unwrap_err(), error);

        let mut toml = toml.to_owned();
        toml += "\nname = \"test\"";
        assert_toml_error(parse_test_container(&toml).unwrap_err(), error);
    }

    fn directive_parse_test(toml: &str, expected: TestDirective<String>) {
        assert_eq!(parse_test_directive(toml).unwrap(), expected);
    }

    fn directive_or_container_parse_test(toml: &str, expected: TestDirective<String>) {
        assert_eq!(parse_test_directive(toml).unwrap(), expected);

        let mut toml = toml.to_owned();
        toml += "\nname = \"test\"";
        assert_eq!(
            parse_test_container(&toml).unwrap().container,
            expected.container
        );
    }

    #[test]
    fn empty() {
        assert_eq!(parse_test_directive("").unwrap(), TestDirective::default(),);
    }

    #[test]
    fn unknown_field() {
        directive_error_test(
            r#"
            unknown = "foo"
            "#,
            "unknown field `unknown`, expected one of",
        );
    }

    #[test]
    fn duplicate_field() {
        directive_error_test(
            r#"
            filter = "all"
            filter = "any"
            "#,
            "duplicate key `filter`",
        );
    }

    #[test]
    fn simple_fields() {
        directive_parse_test(
            r#"
            filter = "package.equals(package1) && test.equals(test1)"
            include_shared_libraries = true
            network = "loopback"
            enable_writable_file_system = true
            user = 101
            group = 202
            timeout = 1
            "#,
            TestDirective {
                filter: Some(
                    "package.equals(package1) && test.equals(test1)"
                        .parse()
                        .unwrap(),
                ),
                include_shared_libraries: Some(true),
                timeout: Some(Timeout::new(1)),
                container: TestContainer {
                    network: Some(JobNetwork::Loopback),
                    enable_writable_file_system: Some(true),
                    user: Some(UserId::from(101)),
                    group: Some(GroupId::from(202)),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn zero_timeout() {
        directive_parse_test(
            r#"
            filter = "package.equals(package1) && test.equals(test1)"
            timeout = 0
            "#,
            TestDirective {
                filter: Some(
                    "package.equals(package1) && test.equals(test1)"
                        .parse()
                        .unwrap(),
                ),
                timeout: Some(None),
                ..Default::default()
            },
        );
    }

    #[test]
    fn mounts() {
        directive_or_container_parse_test(
            indoc! {r#"
                mounts = [
                    { type = "proc", mount_point = "/proc" },
                    { type = "bind", mount_point = "/bind", local_path = "/local" },
                    { type = "bind", mount_point = "/bind2", local_path = "/local2", read_only = true },
                    { type = "devices", devices = ["null", "zero"] },
                ]
            "#},
            TestDirective {
                container: TestContainer {
                    mounts: Some(vec![
                        JobMountForTomlAndJson::Proc {
                            mount_point: non_root_utf8_path_buf!("/proc"),
                        },
                        JobMountForTomlAndJson::Bind {
                            mount_point: non_root_utf8_path_buf!("/bind"),
                            local_path: utf8_path_buf!("/local"),
                            read_only: false,
                        },
                        JobMountForTomlAndJson::Bind {
                            mount_point: non_root_utf8_path_buf!("/bind2"),
                            local_path: utf8_path_buf!("/local2"),
                            read_only: true,
                        },
                        JobMountForTomlAndJson::Devices {
                            devices: enum_set!(
                                JobDeviceForTomlAndJson::Null | JobDeviceForTomlAndJson::Zero
                            ),
                        },
                    ]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn added_mounts() {
        directive_or_container_parse_test(
            indoc! {r#"
                added_mounts = [
                    { type = "proc", mount_point = "/proc" },
                    { type = "bind", mount_point = "/bind", local_path = "/local", read_only = true },
                    { type = "devices", devices = ["null", "zero"] },
                ]
            "#},
            TestDirective {
                container: TestContainer {
                    added_mounts: vec![
                        JobMountForTomlAndJson::Proc {
                            mount_point: non_root_utf8_path_buf!("/proc"),
                        },
                        JobMountForTomlAndJson::Bind {
                            mount_point: non_root_utf8_path_buf!("/bind"),
                            local_path: utf8_path_buf!("/local"),
                            read_only: true,
                        },
                        JobMountForTomlAndJson::Devices {
                            devices: enum_set!(
                                JobDeviceForTomlAndJson::Null | JobDeviceForTomlAndJson::Zero
                            ),
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn mounts_before_added_mounts() {
        directive_or_container_parse_test(
            indoc! {r#"
                mounts = [ { type = "proc", mount_point = "/proc" } ]
                added_mounts = [ { type = "tmp", mount_point = "/tmp" } ]
            "#},
            TestDirective {
                container: TestContainer {
                    mounts: Some(vec![JobMountForTomlAndJson::Proc {
                        mount_point: non_root_utf8_path_buf!("/proc"),
                    }]),
                    added_mounts: vec![JobMountForTomlAndJson::Tmp {
                        mount_point: non_root_utf8_path_buf!("/tmp"),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn mounts_after_added_mounts() {
        directive_or_container_error_test(
            indoc! {r#"
                added_mounts = [ { type = "tmp", mount_point = "/tmp" } ]
                mounts = [ { type = "proc", mount_point = "/proc" } ]
            "#},
            "field `mounts` cannot be set after `added_mounts`",
        );
    }

    #[test]
    fn unknown_field_in_simple_mount() {
        directive_or_container_error_test(
            indoc! {r#"
                mounts = [ { type = "proc", mount_point = "/proc", unknown = "true" } ]
            "#},
            "unknown field `unknown`, expected",
        );
    }

    #[test]
    fn unknown_field_in_bind_mount() {
        directive_or_container_error_test(
            indoc! {r#"
                mounts = [ { type = "bind", mount_point = "/bind", local_path = "/a", unknown = "true" } ]
            "#},
            "unknown field `unknown`, expected",
        );
    }

    #[test]
    fn missing_field_in_simple_mount() {
        directive_or_container_error_test(
            indoc! {r#"
                mounts = [ { type = "proc" } ]
            "#},
            "missing field `mount_point`",
        );
    }

    #[test]
    fn missing_field_in_bind_mount() {
        directive_or_container_error_test(
            indoc! {r#"
                mounts = [ { type = "bind", mount_point = "/bind" } ]
            "#},
            "missing field `local_path`",
        );
    }

    #[test]
    fn missing_flags_field_in_bind_mount_is_okay() {
        directive_or_container_parse_test(
            indoc! {r#"
                mounts = [ { type = "bind", mount_point = "/bind", local_path = "/a" } ]
            "#},
            TestDirective {
                container: TestContainer {
                    mounts: Some(vec![JobMountForTomlAndJson::Bind {
                        mount_point: non_root_utf8_path_buf!("/bind"),
                        local_path: utf8_path_buf!("/a"),
                        read_only: false,
                    }]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn mount_point_of_root_is_disallowed() {
        let mounts = [
            r#"{ type = "bind", mount_point = "/", local_path = "/a" }"#,
            r#"{ type = "proc", mount_point = "/" }"#,
            r#"{ type = "tmp", mount_point = "/" }"#,
            r#"{ type = "sys", mount_point = "/" }"#,
        ];
        for mount in mounts {
            assert!(parse_test_directive(&format!("mounts = [ {mount} ]"))
                .unwrap_err()
                .to_string()
                .contains("a path of \"/\" not allowed"));
            assert!(parse_test_container(&format!("mounts = [ {mount} ]"))
                .unwrap_err()
                .to_string()
                .contains("a path of \"/\" not allowed"));
        }
    }

    #[test]
    fn unknown_devices_type() {
        directive_or_container_error_test(
            r#"
            mounts = [{ type = "devices",  devices = ["unknown"] }]
            "#,
            "unknown variant `unknown`, expected one of",
        );
    }

    #[test]
    fn working_directory() {
        directive_or_container_parse_test(
            r#"
            working_directory = "/foo"
            "#,
            TestDirective {
                container: TestContainer {
                    working_directory: Some(PossiblyImage::Explicit("/foo".into())),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_with_no_use() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust" }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    layers: Some(PossiblyImage::Image),
                    environment: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_as_string() {
        directive_or_container_parse_test(
            r#"
            image = "rust"
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    layers: Some(PossiblyImage::Image),
                    environment: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_with_working_directory() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["layers", "working_directory"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    layers: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn working_directory_after_image_without_working_directory() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["layers"] }
            working_directory = "/foo"
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Explicit("/foo".into())),
                    layers: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_without_working_directory_after_working_directory() {
        directive_or_container_parse_test(
            r#"
            working_directory = "/foo"
            image = { name = "rust", use = ["layers"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Explicit("/foo".into())),
                    layers: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn working_directory_after_image_with_working_directory() {
        directive_or_container_error_test(
            r#"
            image = { name = "rust", use = ["layers", "working_directory"] }
            working_directory = "/foo"
            "#,
            "field `working_directory` cannot be set after `image` field that uses `working_directory`"
        );
    }

    #[test]
    fn image_with_working_directory_after_working_directory() {
        directive_or_container_error_test(
            r#"
            working_directory = "/foo"
            image = { name = "rust", use = ["layers", "working_directory"] }
            "#,
            "field `image` cannot use `working_directory` if field `working_directory` is also set",
        );
    }

    #[test]
    fn layers_tar() {
        directive_or_container_parse_test(
            r#"
            layers = [{ tar = "foo.tar" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![tar_layer!("foo.tar")])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_glob() {
        directive_or_container_parse_test(
            r#"
            layers = [{ glob = "foo*.bin" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![glob_layer!("foo*.bin")])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ glob = "foo*.bin", strip_prefix = "a" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![glob_layer!(
                        "foo*.bin",
                        strip_prefix = "a"
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ glob = "foo*.bin", prepend_prefix = "b" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![glob_layer!(
                        "foo*.bin",
                        prepend_prefix = "b"
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ glob = "foo*.bin", canonicalize = true }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![glob_layer!(
                        "foo*.bin",
                        canonicalize = true
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_paths() {
        directive_or_container_parse_test(
            r#"
            layers = [{ paths = ["foo.bin", "bar.bin"] }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![paths_layer!([
                        "foo.bin", "bar.bin"
                    ])])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ paths = ["foo.bin", "bar.bin"], strip_prefix = "a" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![paths_layer!(
                        ["foo.bin", "bar.bin"],
                        strip_prefix = "a"
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ paths = ["foo.bin", "bar.bin"], prepend_prefix = "a" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![paths_layer!(
                        ["foo.bin", "bar.bin"],
                        prepend_prefix = "a"
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [{ paths = ["foo.bin", "bar.bin"], canonicalize = true }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![paths_layer!(
                        ["foo.bin", "bar.bin"],
                        canonicalize = true
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_stubs() {
        directive_or_container_parse_test(
            r#"
            layers = [{ stubs = ["/foo/bar", "/bin/{baz,qux}/"] }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![LayerSpec::Stubs {
                        stubs: vec!["/foo/bar".into(), "/bin/{baz,qux}/".into()],
                    }])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_symlinks() {
        directive_or_container_parse_test(
            r#"
            layers = [{ symlinks = [{ link = "/hi", target = "/there" }] }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![LayerSpec::Symlinks {
                        symlinks: vec![SymlinkSpec {
                            link: "/hi".into(),
                            target: "/there".into(),
                        }],
                    }])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_shared_library_dependencies() {
        directive_or_container_parse_test(
            r#"
            layers = [
                { shared-library-dependencies = ["/bin/bash", "/bin/sh"] }
            ]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![so_deps_layer!([
                        "/bin/bash",
                        "/bin/sh"
                    ])])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [
                { shared-library-dependencies = ["/bin/bash", "/bin/sh"], prepend_prefix = "/usr" }
            ]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![so_deps_layer!(
                        ["/bin/bash", "/bin/sh"],
                        prepend_prefix = "/usr"
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        directive_or_container_parse_test(
            r#"
            layers = [
                { shared-library-dependencies = ["/bin/bash", "/bin/sh"], canonicalize = true }
            ]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![so_deps_layer!(
                        ["/bin/bash", "/bin/sh"],
                        canonicalize = true
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_with_layers() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["layers", "working_directory"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    layers: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_after_image_without_layers() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["working_directory"] }
            layers = [{ tar = "foo.tar" }]
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    layers: Some(PossiblyImage::Explicit(vec![tar_layer!("foo.tar")])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_without_layers_after_layers() {
        directive_or_container_parse_test(
            r#"
            layers = [{ tar = "foo.tar" }]
            image = { name = "rust", use = ["working_directory"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    layers: Some(PossiblyImage::Explicit(vec![tar_layer!("foo.tar")])),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_after_image_with_layers() {
        directive_or_container_error_test(
            r#"
            image = { name = "rust", use = ["layers", "working_directory"] }
            layers = [{ tar = "foo.tar" }]
            "#,
            "field `layers` cannot be set after `image` field that uses `layers`",
        )
    }

    #[test]
    fn image_with_layers_after_layers() {
        directive_or_container_error_test(
            r#"
            layers = [{ tar = "foo.tar" }]
            image = { name = "rust", use = ["layers", "working_directory"] }
            "#,
            "field `image` cannot use `layers` if field `layers` is also set",
        )
    }

    #[test]
    fn added_layers() {
        directive_or_container_parse_test(
            r#"
            added_layers = [{ tar = "foo.tar" }]
            "#,
            TestDirective {
                container: TestContainer {
                    added_layers: vec![tar_layer!("foo.tar")],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn added_layers_after_layers() {
        directive_or_container_parse_test(
            r#"
            layers = [{ tar = "foo.tar" }]
            added_layers = [{ tar = "bar.tar" }]
            "#,
            TestDirective {
                container: TestContainer {
                    layers: Some(PossiblyImage::Explicit(vec![tar_layer!("foo.tar")])),
                    added_layers: vec![tar_layer!("bar.tar")],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn added_layers_after_image_with_layers() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["layers"] }
            added_layers = [{ tar = "foo.tar" }]
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    layers: Some(PossiblyImage::Image),
                    added_layers: vec![tar_layer!("foo.tar")],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn layers_after_added_layers() {
        directive_or_container_error_test(
            r#"
            added_layers = [{ tar = "bar.tar" }]
            layers = [{ tar = "foo.tar" }]
            "#,
            "field `layers` cannot be set after `added_layers`",
        );
    }

    #[test]
    fn image_with_layers_after_added_layers() {
        directive_or_container_error_test(
            r#"
            added_layers = [{ tar = "bar.tar" }]
            image = { name = "rust", use = ["layers"] }
            "#,
            "field `image` that uses `layers` cannot be set after `added_layers`",
        );
    }

    #[test]
    fn environment() {
        directive_or_container_parse_test(
            r#"
            environment = { FOO = "foo" }
            "#,
            TestDirective {
                container: TestContainer {
                    environment: Some(PossiblyImage::Explicit(BTreeMap::from([(
                        string!("FOO"),
                        string!("foo"),
                    )]))),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_with_environment() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["environment", "working_directory"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    environment: Some(PossiblyImage::Image),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn environment_after_image_without_environment() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["working_directory"] }
            environment = { FOO = "foo" }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    environment: Some(PossiblyImage::Explicit(BTreeMap::from([(
                        string!("FOO"),
                        string!("foo"),
                    )]))),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn image_without_environment_after_environment() {
        directive_or_container_parse_test(
            r#"
            environment = { FOO = "foo" }
            image = { name = "rust", use = ["working_directory"] }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    working_directory: Some(PossiblyImage::Image),
                    environment: Some(PossiblyImage::Explicit(BTreeMap::from([(
                        string!("FOO"),
                        string!("foo"),
                    )]))),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn environment_after_image_with_environment() {
        directive_or_container_error_test(
            r#"
            image = { name = "rust", use = ["environment", "working_directory"] }
            environment = { FOO = "foo" }
            "#,
            "field `environment` cannot be set after `image` field that uses `environment`",
        )
    }

    #[test]
    fn image_with_environment_after_environment() {
        directive_or_container_error_test(
            r#"
            environment = { FOO = "foo" }
            image = { name = "rust", use = ["environment", "working_directory"] }
            "#,
            "field `image` cannot use `environment` if field `environment` is also set",
        )
    }

    #[test]
    fn added_environment() {
        directive_or_container_parse_test(
            r#"
            added_environment = { BAR = "bar" }
            "#,
            TestDirective {
                container: TestContainer {
                    added_environment: BTreeMap::from([(string!("BAR"), string!("bar"))]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn added_environment_after_environment() {
        directive_or_container_parse_test(
            r#"
            environment = { FOO = "foo" }
            added_environment = { BAR = "bar" }
            "#,
            TestDirective {
                container: TestContainer {
                    environment: Some(PossiblyImage::Explicit(BTreeMap::from([(
                        string!("FOO"),
                        string!("foo"),
                    )]))),
                    added_environment: BTreeMap::from([(string!("BAR"), string!("bar"))]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn added_environment_after_image_with_environment() {
        directive_or_container_parse_test(
            r#"
            image = { name = "rust", use = ["environment"] }
            added_environment = { BAR = "bar" }
            "#,
            TestDirective {
                container: TestContainer {
                    image: Some(string!("rust")),
                    environment: Some(PossiblyImage::Image),
                    added_environment: BTreeMap::from([(string!("BAR"), string!("bar"))]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    #[test]
    fn environment_after_added_environment() {
        directive_or_container_error_test(
            r#"
            added_environment = { BAR = "bar" }
            environment = { FOO = "foo" }
            "#,
            "field `environment` cannot be set after `added_environment`",
        );
    }

    #[test]
    fn image_with_environment_after_added_environment() {
        directive_or_container_error_test(
            r#"
            added_environment = { BAR = "bar" }
            image = { name = "rust", use = ["environment"] }
            "#,
            "field `image` that uses `environment` cannot be set after `added_environment`",
        );
    }
}
