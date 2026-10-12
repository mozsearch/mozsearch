//! The documentation of external JVM symbols (ones a SCIP index of Java or
//! Kotlin code uses but doesn't define, see scip-indexer), as structured
//! records' `docs` (see `AnalysisStructured::docs`), only where we know the
//! pages are there.  scip-java's symbols don't say which library they're from
//! (ex: "semanticdb maven . . java/lang/String#"), so it's by their packages,
//! which only one place documents:
//! - `android.*` (but the old support library's `android.support.*`): the
//!   Android platform's reference.
//! - `java.*` (which only the JDK can define) and `javax.*`, of the packages
//!   the platform's reference documents (as Android provides them, which is
//!   what Gecko's Java and Kotlin code, all Android's, uses;
//!   `jvm_docs/android-java-packages.txt`): the same reference.
//! - `androidx.*`: AndroidX's Kotlin reference, which documents every AndroidX
//!   library (its Java reference doesn't have the Kotlin-only ones, ex:
//!   Compose's); a Kotlin file's facade class (ex: `ComposablesKt`) is its
//!   package's page.
//!
//! Classes (and interfaces, nested ones too) get their pages, and packages
//! theirs.  Fields get their anchors on their classes' pages in the platform's
//! reference (AndroidX's anchors are different, ex:
//! "#EXTRA_TITLE()"), but methods only their classes' pages: their anchors
//! have their parameters' types, which SCIP's symbols don't (only
//! disambiguators, ex: `indexOf(+1).`).

use std::collections::HashSet;

use lazy_static::lazy_static;
use ustr::{Ustr, ustr};

use crate::file_format::analysis::DocLink;

lazy_static! {
    /// The `java.*` and `javax.*` packages the Android platform's reference
    /// documents.
    static ref ANDROID_JAVA_PACKAGES: HashSet<&'static str> =
        include_str!("jvm_docs/android-java-packages.txt")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .collect();
}

/// A piece of a JVM symbol, from a SCIP symbol's descriptors.
#[derive(Clone, Copy, Debug)]
pub enum Piece<'a> {
    Package(&'a str),
    Type(&'a str),
    /// A field (or a Kotlin property).
    Term(&'a str),
    Method(&'a str),
}

/// The documentation of the external JVM symbol with `pieces` (its package,
/// then its type and nested types, then perhaps a member), if we know where it
/// is.
pub fn jvm_docs(pieces: &[Piece]) -> Vec<DocLink<Ustr>> {
    let mut packages = vec![];
    let mut types = vec![];
    let mut member = None;
    for piece in pieces {
        match (*piece, types.is_empty(), member) {
            (Piece::Package(name), true, None) => packages.push(name),
            (Piece::Type(name), _, None) => types.push(name),
            (Piece::Term(_) | Piece::Method(_), false, None) => member = Some(*piece),
            _ => return vec![],
        }
    }
    if packages.len() < 2 {
        return vec![];
    }
    let package = packages.join(".");
    let path = packages.join("/");
    let link = |site: &str, url: String| {
        vec![DocLink {
            site: ustr(site),
            url: ustr(&url),
        }]
    };
    let field = match member {
        Some(Piece::Term(name)) => Some(name),
        _ => None,
    };
    let platform = match packages[0] {
        "android" => packages[1] != "support",
        "java" | "javax" => ANDROID_JAVA_PACKAGES.contains(package.as_str()),
        _ => false,
    };
    match packages[0] {
        _ if platform => {
            let base = format!("https://developer.android.com/reference/{}/", path);
            match (types.is_empty(), field) {
                (true, _) => link("android", format!("{}package-summary", base)),
                (false, Some(field)) => {
                    link("android", format!("{}{}#{}", base, types.join("."), field))
                }
                (false, None) => link("android", format!("{}{}", base, types.join("."))),
            }
        }
        "androidx" => {
            let base = format!("https://developer.android.com/reference/kotlin/{}/", path);
            // (A Kotlin file's facade class's members are its package's.)
            let facade = types.len() == 1 && types[0].ends_with("Kt");
            if types.is_empty() || facade {
                link("android", format!("{}package-summary", base))
            } else {
                link("android", format!("{}{}", base, types.join(".")))
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Piece::*;

    fn url(pieces: &[Piece]) -> Option<String> {
        let docs = jvm_docs(pieces);
        assert!(docs.len() <= 1);
        docs.first().map(|doc| format!("{} {}", doc.site, doc.url))
    }

    #[test]
    fn test_java() {
        let reference = "https://developer.android.com/reference";
        assert_eq!(
            url(&[Package("java"), Package("lang"), Type("String")]),
            Some(format!("android {}/java/lang/String", reference))
        );
        assert_eq!(
            url(&[Package("java"), Package("util"), Type("Map"), Type("Entry")]),
            Some(format!("android {}/java/util/Map.Entry", reference))
        );
        assert_eq!(
            url(&[
                Package("java"),
                Package("lang"),
                Type("Integer"),
                Term("MAX_VALUE")
            ]),
            Some(format!("android {}/java/lang/Integer#MAX_VALUE", reference))
        );
        // (A method's is its class's page.)
        assert_eq!(
            url(&[
                Package("java"),
                Package("lang"),
                Type("String"),
                Method("indexOf")
            ]),
            Some(format!("android {}/java/lang/String", reference))
        );
        assert_eq!(
            url(&[Package("java"), Package("lang")]),
            Some(format!("android {}/java/lang/package-summary", reference))
        );
        assert_eq!(
            url(&[Package("javax"), Package("crypto"), Type("Cipher")]),
            Some(format!("android {}/javax/crypto/Cipher", reference))
        );
        // Not packages Android doesn't have (ex: AWT's, Swing's), or which
        // aren't the platform's (ex: JSR-305's), nor the root namespace.
        assert_eq!(
            url(&[Package("javax"), Package("swing"), Type("JFrame")]),
            None
        );
        assert_eq!(url(&[Package("java"), Package("awt"), Type("Color")]), None);
        assert_eq!(
            url(&[Package("javax"), Package("annotation"), Type("Nonnull")]),
            None
        );
        assert_eq!(url(&[Package("java")]), None);
    }

    #[test]
    fn test_android() {
        let reference = "https://developer.android.com/reference";
        assert_eq!(
            url(&[
                Package("android"),
                Package("app"),
                Type("Activity"),
                Term("RESULT_OK")
            ]),
            Some(format!(
                "android {}/android/app/Activity#RESULT_OK",
                reference
            ))
        );
        assert_eq!(
            url(&[
                Package("android"),
                Package("view"),
                Type("View"),
                Type("OnClickListener")
            ]),
            Some(format!(
                "android {}/android/view/View.OnClickListener",
                reference
            ))
        );
        assert_eq!(
            url(&[
                Package("android"),
                Package("support"),
                Package("v4"),
                Type("X")
            ]),
            None
        );
        // AndroidX's Kotlin reference, without fields' anchors, and facade
        // classes' members on their packages' pages.
        assert_eq!(
            url(&[
                Package("androidx"),
                Package("core"),
                Package("app"),
                Type("NotificationCompat"),
                Term("EXTRA_TITLE")
            ]),
            Some(format!(
                "android {}/kotlin/androidx/core/app/NotificationCompat",
                reference
            ))
        );
        assert_eq!(
            url(&[
                Package("androidx"),
                Package("compose"),
                Package("runtime"),
                Type("ComposablesKt"),
                Method("remember")
            ]),
            Some(format!(
                "android {}/kotlin/androidx/compose/runtime/package-summary",
                reference
            ))
        );
        // Not other packages, or odd symbols.
        assert_eq!(
            url(&[Package("org"), Package("mozilla"), Type("GeckoApp")]),
            None
        );
        assert_eq!(url(&[Package("java"), Package("lang"), Term("x")]), None);
    }
}
