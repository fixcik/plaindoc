//! ODF namespace URIs (1.2/1.3) and an element matcher.

use roxmltree::Node;

pub(crate) const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub(crate) const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub(crate) const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub(crate) const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
pub(crate) const DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub(crate) const SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
pub(crate) const DC: &str = "http://purl.org/dc/elements/1.1/";
pub(crate) const META: &str = "urn:oasis:names:tc:opendocument:xmlns:meta:1.0";
pub(crate) const CALCEXT: &str =
    "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0";

/// True for an element with the given namespace and local name.
pub(crate) fn is(n: &Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(ns) && n.tag_name().name() == name
}
