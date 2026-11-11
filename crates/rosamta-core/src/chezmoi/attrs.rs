//! chezmoi's source state attributes: the prefixes and suffixes a source
//! name carries, parsed in chezmoi's fixed order.

/// What a source file stands for in the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Regular,
    Create,
    Modify,
    Remove,
    Symlink,
    Script(ScriptAttrs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptAttrs {
    /// `once_` or `onchange_`; neither means every apply.
    pub run: Option<ScriptRun>,
    pub before: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptRun {
    Once,
    Onchange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileAttrs {
    pub kind: FileKind,
    pub encrypted: bool,
    pub private: bool,
    pub readonly: bool,
    pub empty: bool,
    pub executable: bool,
    pub template: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirAttrs {
    pub remove: bool,
    pub external: bool,
    pub exact: bool,
    pub private: bool,
    pub readonly: bool,
}

/// Strips prefixes in order until `literal_` or the first one missing.
struct Prefixes<'a> {
    name: &'a str,
    literal: bool,
}

impl<'a> Prefixes<'a> {
    fn new(name: &'a str) -> Self {
        Self {
            name,
            literal: false,
        }
    }

    fn take(&mut self, prefix: &str) -> bool {
        if self.literal {
            return false;
        }
        if let Some(rest) = self.name.strip_prefix("literal_") {
            self.name = rest;
            self.literal = true;
            return false;
        }
        match self.name.strip_prefix(prefix) {
            Some(rest) => {
                self.name = rest;
                true
            }
            None => false,
        }
    }

    /// The target name: `dot_` last.
    fn finish(mut self) -> String {
        dotted(self.take("dot_"), self.name)
    }
}

fn dotted(dot: bool, name: &str) -> String {
    if dot {
        format!(".{name}")
    } else {
        name.to_string()
    }
}

pub fn parse_dir(name: &str) -> (String, DirAttrs) {
    let mut name = Prefixes::new(name);
    let attrs = DirAttrs {
        remove: name.take("remove_"),
        external: name.take("external_"),
        exact: name.take("exact_"),
        private: name.take("private_"),
        readonly: name.take("readonly_"),
    };
    (name.finish(), attrs)
}

/// `age` is whether chezmoi encrypts with age, whose files end in `.age`.
pub fn parse_file(name: &str, age: bool) -> (String, FileAttrs) {
    let mut name = Prefixes::new(name);
    let kind = if name.take("create_") {
        FileKind::Create
    } else if name.take("modify_") {
        FileKind::Modify
    } else if name.take("remove_") {
        FileKind::Remove
    } else if name.take("run_") {
        let run = if name.take("once_") {
            Some(ScriptRun::Once)
        } else if name.take("onchange_") {
            Some(ScriptRun::Onchange)
        } else {
            None
        };
        let before = name.take("before_");
        if !before {
            name.take("after_");
        }
        FileKind::Script(ScriptAttrs { run, before })
    } else if name.take("symlink_") {
        FileKind::Symlink
    } else {
        FileKind::Regular
    };
    let mut attrs = FileAttrs {
        kind,
        encrypted: false,
        private: false,
        readonly: false,
        empty: false,
        executable: false,
        template: false,
    };
    // Scripts and symlinks take no further prefixes but `dot_`.
    if matches!(
        kind,
        FileKind::Regular | FileKind::Create | FileKind::Modify
    ) {
        attrs.encrypted = name.take("encrypted_");
        attrs.private = name.take("private_");
        attrs.readonly = name.take("readonly_");
        if kind != FileKind::Modify {
            attrs.empty = name.take("empty_");
        }
        attrs.executable = name.take("executable_");
    }
    let dot = !matches!(kind, FileKind::Script(_)) && name.take("dot_");
    // The encryption suffix comes off before the others.
    let mut rest = name.name;
    if attrs.encrypted && age {
        rest = rest.strip_suffix(".age").unwrap_or(rest);
    }
    let (rest, template) = suffixes(rest);
    attrs.template = template;
    (dotted(dot, rest), attrs)
}

/// The name without `.tmpl`, read from the end until `.literal`.
fn suffixes(name: &str) -> (&str, bool) {
    let mut name = name;
    let mut template = false;
    loop {
        if let Some(rest) = name.strip_suffix(".literal") {
            return (rest, template);
        }
        match name.strip_suffix(".tmpl") {
            Some(rest) if !template => {
                name = rest;
                template = true;
            }
            _ => return (name, template),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str) -> String {
        parse_file(name, true).0
    }

    /// Each case was checked against `chezmoi managed` 2.x.
    #[test]
    fn names_follow_chezmoi() {
        for (source, expected) in [
            ("literal_dot_a", "dot_a"),
            ("private_literal_dot_b", "dot_b"),
            ("dot_literal_c", ".literal_c"),
            ("private_dot_d.literal", ".d"),
            ("private_dot_e.tmpl.literal", ".e.tmpl"),
            ("dot_f.tmpl", ".f"),
            ("executable_private_dot_g", "private_dot_g"),
            ("private_executable_dot_h", ".h"),
            ("empty_dot_i", ".i"),
            ("encrypted_private_dot_j.age", ".j"),
            ("encrypted_dot_x.tmpl.age", ".x"),
            ("encrypted_dot_y.age.tmpl", ".y.age"),
            ("readonly_dot_k", ".k"),
            ("create_private_dot_l", ".l"),
            ("modify_dot_m", ".m"),
            ("symlink_dot_n", ".n"),
            ("remove_dot_o", ".o"),
            ("dot_p.literal.tmpl", ".p"),
            ("run_once_before_10-x.sh", "10-x.sh"),
            ("run_onchange_y.sh.tmpl", "y.sh"),
            ("run_z.sh", "z.sh"),
            ("README.md", "README.md"),
        ] {
            assert_eq!(target(source), expected, "{source}");
        }
        assert_eq!(parse_file("encrypted_dot_j.age", false).0, ".j.age");
    }

    #[test]
    fn attributes_follow_chezmoi() {
        let (_, attrs) = parse_file(
            "create_encrypted_private_readonly_empty_executable_dot_x.tmpl",
            true,
        );
        assert_eq!(
            attrs,
            FileAttrs {
                kind: FileKind::Create,
                encrypted: true,
                private: true,
                readonly: true,
                empty: true,
                executable: true,
                template: true,
            }
        );
        let script = |name| parse_file(name, true).1.kind;
        assert_eq!(
            script("run_once_before_a.sh"),
            FileKind::Script(ScriptAttrs {
                run: Some(ScriptRun::Once),
                before: true
            })
        );
        assert_eq!(
            script("run_onchange_after_a.sh"),
            FileKind::Script(ScriptAttrs {
                run: Some(ScriptRun::Onchange),
                before: false
            })
        );
        assert_eq!(
            script("run_a.sh"),
            FileKind::Script(ScriptAttrs {
                run: None,
                before: false
            })
        );
        // A script keeps a name that looks like an attribute.
        assert_eq!(target("run_once_dot_a.sh"), "dot_a.sh");
    }

    #[test]
    fn directories_follow_chezmoi() {
        assert_eq!(
            parse_dir("exact_private_dot_ssh"),
            (
                ".ssh".to_string(),
                DirAttrs {
                    exact: true,
                    private: true,
                    ..Default::default()
                }
            )
        );
        assert_eq!(parse_dir("readonly_private_dot_t").0, "private_dot_t");
        assert!(parse_dir("external_dot_r").1.external);
        assert_eq!(parse_dir("literal_exact_x").0, "exact_x");
    }
}
