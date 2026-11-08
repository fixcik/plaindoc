//! Counting tree sink: bounds the work html5ever's tree builder does (R09).

use std::borrow::Cow;
use std::cell::Ref;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use html5ever::driver;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, QualName};
use plaindoc_core::{ParseError, Result};
use scraper::{Html, HtmlTreeSink};

/// calls into the sink once per element, so the count is the builder's real work.
/// Normal pages take ~1.2 per byte, a 50 000-row 1C table ~1.8; quadratic shapes
/// (deep stacks scanned by every tag) take hundreds to thousands per byte.
const WORK_PER_BYTE: u64 = 32;
/// Callbacks allowed on top of [`WORK_PER_BYTE`]: room for short deep documents
/// (4 000 nested lists take 6.4e7); ~0.5 s release, ~6 s debug at the cap.
const WORK_BASE: u64 = 130_000_000;

/// Unwind payload of [`CountingSink`] over its limit.
struct WorkExceeded;

/// scraper's sink that counts every callback and unwinds past `limit`.
struct CountingSink {
    inner: HtmlTreeSink,
    work: std::cell::Cell<u64>,
    limit: u64,
}

impl CountingSink {
    fn tick(&self) {
        let n = self.work.get() + 1;
        self.work.set(n);
        if n > self.limit {
            // not `panic!`: no panic hook output, caught in `parse_html`
            resume_unwind(Box::new(WorkExceeded));
        }
    }
}

impl TreeSink for CountingSink {
    type Handle = <HtmlTreeSink as TreeSink>::Handle;
    type Output = Html;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Html {
        self.inner.finish()
    }
    fn parse_error(&self, msg: Cow<'static, str>) {
        self.tick();
        self.inner.parse_error(msg)
    }
    fn get_document(&self) -> Self::Handle {
        self.tick();
        self.inner.get_document()
    }
    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Ref<'a, QualName> {
        self.tick();
        self.inner.elem_name(target)
    }
    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        self.tick();
        self.inner.create_element(name, attrs, flags)
    }
    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.tick();
        self.inner.create_comment(text)
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.tick();
        self.inner.create_pi(target, data)
    }
    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.tick();
        self.inner.append(parent, child)
    }
    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.tick();
        self.inner
            .append_based_on_parent_node(element, prev_element, child)
    }
    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.tick();
        self.inner
            .append_doctype_to_document(name, public_id, system_id)
    }
    fn mark_script_already_started(&self, node: &Self::Handle) {
        self.tick();
        self.inner.mark_script_already_started(node)
    }
    fn pop(&self, node: &Self::Handle) {
        self.tick();
        self.inner.pop(node)
    }
    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.tick();
        self.inner.get_template_contents(target)
    }
    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        self.tick();
        self.inner.same_node(x, y)
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.tick();
        self.inner.set_quirks_mode(mode)
    }
    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        self.tick();
        self.inner.append_before_sibling(sibling, new_node)
    }
    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        self.tick();
        self.inner.add_attrs_if_missing(target, attrs)
    }
    fn associate_with_form(
        &self,
        target: &Self::Handle,
        form: &Self::Handle,
        nodes: (&Self::Handle, Option<&Self::Handle>),
    ) {
        self.tick();
        self.inner.associate_with_form(target, form, nodes)
    }
    fn remove_from_parent(&self, target: &Self::Handle) {
        self.tick();
        self.inner.remove_from_parent(target)
    }
    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.tick();
        self.inner.reparent_children(node, new_parent)
    }
    fn is_mathml_annotation_xml_integration_point(&self, handle: &Self::Handle) -> bool {
        self.tick();
        self.inner
            .is_mathml_annotation_xml_integration_point(handle)
    }
    fn set_current_line(&self, line_number: u64) {
        self.tick();
        self.inner.set_current_line(line_number)
    }
    fn allow_declarative_shadow_roots(&self, intended_parent: &Self::Handle) -> bool {
        self.tick();
        self.inner.allow_declarative_shadow_roots(intended_parent)
    }
    fn attach_declarative_shadow(
        &self,
        location: &Self::Handle,
        template: &Self::Handle,
        attrs: &[Attribute],
    ) -> bool {
        self.tick();
        self.inner
            .attach_declarative_shadow(location, template, attrs)
    }
    fn maybe_clone_an_option_into_selectedcontent(&self, option: &Self::Handle) {
        self.tick();
        self.inner
            .maybe_clone_an_option_into_selectedcontent(option)
    }
}

/// `Html::parse_document` with the work cap: `Limit { "html parser work", limit }`
/// past `WORK_BASE + WORK_PER_BYTE * len` sink callbacks.
pub(super) fn parse_html(text: &str) -> Result<Html> {
    let limit = WORK_PER_BYTE
        .saturating_mul(text.len() as u64)
        .saturating_add(WORK_BASE);
    let sink = CountingSink {
        inner: HtmlTreeSink::new(Html::new_document()),
        work: std::cell::Cell::new(0),
        limit,
    };
    match catch_unwind(AssertUnwindSafe(|| {
        driver::parse_document(sink, Default::default()).one(text)
    })) {
        Ok(doc) => Ok(doc),
        Err(p) if p.is::<WorkExceeded>() => Err(ParseError::Limit {
            kind: "html parser work".into(),
            limit,
        }),
        Err(p) => resume_unwind(p),
    }
}
