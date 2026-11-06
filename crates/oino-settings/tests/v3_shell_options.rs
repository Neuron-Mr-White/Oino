//! Execution-option regressions for the conservative shell risk floor.
use oino_settings::autonomy::{Level, bash_floor};

#[test]
fn execution_options_are_high() {
    for command in [
        "fd -xrm marker",
        "fd --exec-batch=rm marker",
        "fd -x rm marker",
        "fd -X rm marker",
        "fd -Xrm marker",
        "fd -Hxrm marker",
        "fd -HXrm marker",
        "fd --exec=rm marker",
        "fd --exec-batch rm marker",
        "find . -exec rm marker \\;",
        "find . -execrm marker \\;",
        "find . -execdir rm marker \\;",
        "find . -execdir=rm marker \\;",
        "find . -delete",
        "find . -delete=marker",
        "find . -ok rm marker \\;",
        "find . -okrm marker \\;",
        "find . -okdir=rm marker \\;",
        "xargs rm marker",
        "xargs -I{} rm marker",
        "xargs --replace={} rm marker",
        "git -c core.pager=rm log",
        "git -ccore.pager=rm log",
        "git --config-env=core.pager=PAGER log",
        "env -S 'rm marker'",
        "env -Srm marker",
        "env --split-string='rm marker'",
    ] {
        assert_eq!(bash_floor(command), Level::High, "{command}");
    }
}

#[test]
fn read_only_options_stay_low() {
    for command in [
        "fd marker",
        "fd -HI marker",
        "fd --extension rs marker",
        "find . -name marker -print",
        "git status",
        "git diff",
    ] {
        assert_eq!(bash_floor(command), Level::Low, "{command}");
    }
}
