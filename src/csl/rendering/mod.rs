use std::borrow::Cow;
use std::fmt::Write;
use std::str::FromStr;

use citationberg::taxonomy::{
    NumberVariable, OtherTerm, StandardVariable, Term, Variable,
};
use citationberg::{
    ChooseBranch, CslMacro, DateDayForm, DateMonthForm, DatePartName, DateParts,
    DateStrongAnyForm, LabelPluralize, LayoutRenderingElement, LongShortForm, NumberForm,
    TestPosition, TextCase, ToAffixes, ToFormatting,
};
use citationberg::{TermForm, TextTarget};

use crate::lang::{Case, SentenceCase, TitleCase};
use crate::types::{ChunkedString, Date, MaybeTyped, Numeric};

use super::taxonomy::EntryLike;
use super::{Context, ElemMeta, IbidState, SpecialForm};

pub mod names;

/// All rendering elements implement this trait. It allows you to format an
/// [`Entry`] with them.
pub(crate) trait RenderCsl {
    /// Render the element given the context's Entry into the context's buffer.
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>);
}

impl RenderCsl for citationberg::Text {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        enum ResolvedTextTarget<'a> {
            StandardVariable(StandardVariable, Cow<'a, ChunkedString>),
            NumberVariable(NumberVariable, MaybeTyped<Cow<'a, Numeric>>),
            Macro(&'a CslMacro),
            Term(&'a str),
            Value(&'a str),
        }

        if ctx.instance.kind == Some(SpecialForm::AuthorOnly)
            && !matches!(&self.target, TextTarget::Macro { .. })
        {
            return;
        }

        let Some(target) = (match &self.target {
            TextTarget::Variable { var: Variable::Standard(var), form } => ctx
                .resolve_standard_variable(*form, *var)
                .map(|s| ResolvedTextTarget::StandardVariable(*var, s)),
            TextTarget::Variable { var: Variable::Number(var), .. } => ctx
                .resolve_number_variable(*var)
                .map(|n| ResolvedTextTarget::NumberVariable(*var, n)),
            TextTarget::Variable { .. } => None,
            TextTarget::Macro { name } => {
                ctx.style.get_macro(name).map(ResolvedTextTarget::Macro)
            }
            TextTarget::Term { term, form, plural } => {
                ctx.term(*term, *form, *plural).map(ResolvedTextTarget::Term)
            }
            TextTarget::Value { val } => Some(ResolvedTextTarget::Value(val)),
        }) else {
            return;
        };

        let depth = ctx.push_elem(self.formatting);

        let affix_loc = ctx.apply_prefix(&self.affixes);

        if self.quotes {
            ctx.push_quotes();
        }

        ctx.may_strip_periods(self.strip_periods);
        let cidx = ctx.push_case(self.text_case);

        match target {
            ResolvedTextTarget::StandardVariable(var, val) => match var {
                StandardVariable::URL => {
                    let str = val.to_string();
                    ctx.push_link(&val, str);
                }
                StandardVariable::DOI => {
                    let url = format!("https://doi.org/{}", val.to_str());
                    ctx.push_link(&val, url);
                }
                StandardVariable::PMID => {
                    let url =
                        format!("https://www.ncbi.nlm.nih.gov/pubmed/{}", val.to_str());
                    ctx.push_link(&val, url);
                }
                StandardVariable::PMCID => {
                    let url = format!(
                        "https://www.ncbi.nlm.nih.gov/pmc/articles/{}",
                        val.to_str()
                    );
                    ctx.push_link(&val, url);
                }
                _ => ctx.push_chunked(&val),
            },
            ResolvedTextTarget::NumberVariable(_, n) => ctx.push_str(&n.to_str()),
            ResolvedTextTarget::Macro(mac) => {
                let len = ctx.writing.len();

                for child in &mac.children {
                    child.render(ctx);
                }

                if len < ctx.writing.len() {
                    ctx.writing.printed_non_empty_macro();
                }
            }
            ResolvedTextTarget::Term(s) => ctx.push_str(s),
            ResolvedTextTarget::Value(val) => ctx.push_str(val),
        }

        ctx.pop_case(cidx);
        ctx.stop_stripping_periods();

        if self.quotes {
            ctx.pop_quotes();
            ctx.may_pull_punctuation();
        }

        ctx.apply_suffix(&self.affixes, affix_loc);
        ctx.commit_elem(
            depth,
            self.display,
            matches!(&self.target, TextTarget::Variable { .. }).then_some(ElemMeta::Text),
        );
    }
}

impl RenderCsl for citationberg::Number {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        if ctx.instance.kind == Some(SpecialForm::AuthorOnly) {
            return;
        }

        let value = ctx.resolve_number_variable(self.variable);
        if ctx.instance.sorting {
            if let Some(MaybeTyped::Typed(n)) = value {
                n.fmt_value(ctx, true).unwrap();
                return;
            }
        }

        let depth = ctx.push_elem(self.formatting);
        let affix_loc = ctx.apply_prefix(&self.affixes);
        let cidx = ctx.push_case(self.text_case);
        let gender = ctx.gender(self.variable.into());

        match value {
            Some(MaybeTyped::Typed(num)) if num.will_transform() => {
                let normal_num = if self.form == NumberForm::Numeric
                    && self.variable == NumberVariable::Page
                {
                    if let Some(range) = num.range() {
                        ctx.style
                            .csl
                            .settings
                            .page_range_format
                            .unwrap_or_default()
                            .format(
                                range,
                                ctx,
                                ctx.term(
                                    OtherTerm::PageRangeDelimiter.into(),
                                    TermForm::default(),
                                    false,
                                )
                                .or(Some("–")),
                            )
                            .unwrap();
                        false
                    } else {
                        true
                    }
                } else {
                    true
                };

                if normal_num {
                    num.as_ref()
                        .with_form(ctx, self.form, gender, ctx.ordinal_lookup())
                        .unwrap();
                }
            }
            Some(MaybeTyped::Typed(num)) => write!(ctx, "{}", num).unwrap(),
            Some(MaybeTyped::String(s)) => ctx.push_str(&s),
            None => {}
        }

        ctx.pop_case(cidx);
        ctx.apply_suffix(&self.affixes, affix_loc);
        ctx.commit_elem(
            depth,
            self.display,
            (self.variable == NumberVariable::CitationNumber)
                .then_some(ElemMeta::CitationNumber)
                .or(Some(ElemMeta::Number)),
        );
    }
}

impl RenderCsl for citationberg::Label {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        if ctx.instance.kind == Some(SpecialForm::AuthorOnly) {
            return;
        }

        let Some(variable) = ctx.resolve_number_variable(self.variable) else {
            return;
        };

        let depth = ctx.push_elem(citationberg::Formatting::default());
        let plural = match self.label.plural {
            LabelPluralize::Always => true,
            LabelPluralize::Never => false,
            LabelPluralize::Contextual => match variable {
                MaybeTyped::String(_) => false,
                MaybeTyped::Typed(n) => {
                    n.is_plural(self.variable.is_number_of_variable())
                }
            },
        };

        let content = ctx
            .term(Term::from(self.variable), self.label.form, plural)
            .unwrap_or_default();

        render_label_with_var(&self.label, ctx, content);
        ctx.commit_elem(depth, None, Some(ElemMeta::Label));
    }
}

fn render_label_with_var<T: EntryLike>(
    label: &citationberg::VariablelessLabel,
    ctx: &mut Context<T>,
    content: &str,
) {
    if content.is_empty() {
        return;
    }

    let idx = ctx.push_format(label.formatting);

    let affixes = &label.affixes;
    let affix_loc = ctx.apply_prefix(affixes);

    ctx.may_strip_periods(label.strip_periods);
    let cidx = ctx.push_case(label.text_case);

    ctx.push_str(content);

    ctx.pop_case(cidx);
    ctx.stop_stripping_periods();
    ctx.apply_suffix(affixes, affix_loc);
    ctx.pop_format(idx);
}

impl RenderCsl for citationberg::Date {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        if ctx.instance.kind == Some(SpecialForm::AuthorOnly) {
            return;
        }

        let Some(variable) = self.variable else { return };
        let Some(date) = ctx.resolve_date_variable(variable) else { return };

        if ctx.instance.sorting {
            let year;
            let mut month = false;
            let mut day = false;

            if self.is_localized() {
                match self.parts {
                    Some(DateParts::Year) => year = true,
                    Some(DateParts::YearMonth) => {
                        year = true;
                        month = true;
                    }
                    Some(DateParts::YearMonthDay) | None => {
                        year = true;
                        month = true;
                        day = true;
                    }
                }
            } else {
                year = self.date_part.iter().any(|i| i.name == DatePartName::Year);
                month = self.date_part.iter().any(|i| i.name == DatePartName::Month);
                day = self.date_part.iter().any(|i| i.name == DatePartName::Day);
            };

            if year {
                write!(ctx, "{:04}", date.year).unwrap();
                render_year_suffix_implicitly(ctx);
            }

            if month {
                write!(
                    ctx,
                    "{:02}",
                    date.month.map(|m| m as i32 + 1).unwrap_or_default()
                )
                .unwrap();
            }

            if day {
                write!(ctx, "{:02}", date.day.map(|d| d as i32 + 1).unwrap_or_default())
                    .unwrap();
            }

            return;
        }

        let base = if let Some(form) = self.form {
            let Some(base) = ctx.localized_date(form) else { return };
            Some(base)
        } else {
            None
        };

        let formatting = base
            .map(|b| self.formatting.apply(b.formatting))
            .unwrap_or(self.formatting);
        let depth = ctx.push_elem(formatting);

        let affix_loc = ctx.apply_prefix(&self.affixes);

        let cidx = ctx.push_case(self.text_case.or(base.and_then(|b| b.text_case)));

        let parts = self.parts.or(base.and_then(|b| b.parts)).unwrap_or_default();

        // TODO: Date ranges
        let mut last_was_empty = true;
        for part in &base.unwrap_or(self).date_part {
            match part.name {
                DatePartName::Month if !parts.has_month() => continue,
                DatePartName::Day if !parts.has_day() => continue,
                _ => {}
            }

            let cursor = ctx.writing.len();
            if !last_was_empty {
                if let Some(delim) = &self.delimiter {
                    ctx.push_str(delim);
                }
            }

            let over_ride = base
                .is_some()
                .then(|| self.date_part.iter().find(|p| p.name == part.name))
                .flatten();

            render_date_part(part, &date, ctx, over_ride);
            last_was_empty = cursor == ctx.writing.len();
        }

        ctx.pop_case(cidx);
        ctx.apply_suffix(&self.affixes, affix_loc);
        ctx.commit_elem(depth, self.display, Some(ElemMeta::Date));
    }
}

fn render_date_part<T: EntryLike>(
    date_part: &citationberg::DatePart,
    date: &Date,
    ctx: &mut Context<T>,
    over_ride: Option<&citationberg::DatePart>,
) {
    let Some(val) = (match date_part.name {
        DatePartName::Day => date.day.map(|i| i as i32 + 1),
        DatePartName::Month => date.month.map(|i| i as i32 + 1),
        DatePartName::Year => Some(date.year),
    }) else {
        return;
    };

    let formatting = over_ride
        .map(|p| p.formatting.apply(date_part.formatting))
        .unwrap_or(date_part.formatting);

    let idx = ctx.push_format(formatting);

    let affixes = &date_part.affixes;
    let affix_loc = ctx.apply_prefix(affixes);

    if date_part.name == DatePartName::Month {
        ctx.may_strip_periods(date_part.strip_periods);
    }

    let cidx = ctx.push_case(over_ride.and_then(|o| o.text_case).or(date_part.text_case));

    let form = over_ride
        .map(citationberg::DatePart::form)
        .unwrap_or_else(|| date_part.form());
    match form {
        DateStrongAnyForm::Day(DateDayForm::NumericLeadingZeros)
        | DateStrongAnyForm::Month(DateMonthForm::NumericLeadingZeros) => {
            write!(ctx, "{:02}", val).unwrap();
        }
        DateStrongAnyForm::Day(DateDayForm::Ordinal)
            if val != 1
                || !ctx
                    .style
                    .lookup_locale(|l| {
                        Some(
                            l.style_options
                                .and_then(|o| o.limit_day_ordinals_to_day_1)
                                .unwrap_or_default(),
                        )
                    })
                    .unwrap_or_default() =>
        {
            let gender = date
                .month
                .and_then(OtherTerm::month)
                .and_then(|m| ctx.gender(m.into()));

            write!(
                ctx,
                "{}{}",
                val,
                ctx.ordinal_lookup().lookup(val, gender).unwrap_or_default()
            )
            .unwrap();
        }
        DateStrongAnyForm::Day(DateDayForm::Numeric | DateDayForm::Ordinal)
        | DateStrongAnyForm::Month(DateMonthForm::Numeric) => {
            write!(ctx, "{}", val).unwrap();
        }
        DateStrongAnyForm::Month(DateMonthForm::Long) => {
            if let Some(month) = OtherTerm::month((val - 1) as u8)
                .and_then(|m| ctx.term(m.into(), TermForm::Long, false))
            {
                ctx.push_str(month);
            } else {
                write!(ctx, "{}", val).unwrap();
            }
        }
        DateStrongAnyForm::Month(DateMonthForm::Short) => {
            if let Some(month) = OtherTerm::month((val - 1) as u8)
                .and_then(|m| ctx.term(m.into(), TermForm::Short, false))
            {
                ctx.push_str(month);
            } else {
                write!(ctx, "{}", val).unwrap();
            }
        }
        DateStrongAnyForm::Year(LongShortForm::Short) => {
            write!(ctx, "{:02}", (val % 100).abs()).unwrap();
        }
        DateStrongAnyForm::Year(LongShortForm::Long) => {
            write!(ctx, "{}", val.abs()).unwrap();
        }
    }

    if let DateStrongAnyForm::Year(_) = form {
        if val < 1000 {
            ctx.push_str(if val < 0 { "BC" } else { "AD" });
        }
        render_year_suffix_implicitly(ctx);
    }

    ctx.apply_suffix(affixes, affix_loc);
    ctx.stop_stripping_periods();
    ctx.pop_case(cidx);
    ctx.pop_format(idx);
}

/// Render the year suffix if it is set and the style will not render it
/// explicitly.
fn render_year_suffix_implicitly<T: EntryLike>(ctx: &mut Context<T>) {
    if ctx.style.renders_year_suffix_implicitly() {
        if let Some(year_suffix) = ctx.resolve_standard_variable(
            LongShortForm::default(),
            StandardVariable::YearSuffix,
        ) {
            ctx.push_chunked(year_suffix.as_ref());
        }
    }
}

impl RenderCsl for citationberg::Choose {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        for branch in self.branches() {
            if branch.match_.test(BranchConditionIter::from_branch(branch, ctx)) {
                render_with_delimiter(&branch.children, self.delimiter.as_deref(), ctx);
                return;
            }
        }

        if let Some(fallthrough) = &self.otherwise {
            render_with_delimiter(&fallthrough.children, self.delimiter.as_deref(), ctx);
        }
    }
}

fn render_with_delimiter<T: EntryLike>(
    children: &[LayoutRenderingElement],
    delimiter: Option<&str>,
    ctx: &mut Context<T>,
) {
    let mut last_empty = true;
    let mut loc = None;

    for child in children {
        if !last_empty {
            if let Some(delim) = delimiter {
                let prev_loc = std::mem::take(&mut loc);

                if let Some(prev_loc) = prev_loc {
                    ctx.commit_elem(prev_loc, None, None);
                }

                loc = Some(ctx.push_elem(citationberg::Formatting::default()));
                ctx.push_str(delim);
            }
        }

        let pos = ctx.push_elem(citationberg::Formatting::default());

        match child {
            LayoutRenderingElement::Text(text) => text.render(ctx),
            LayoutRenderingElement::Number(num) => num.render(ctx),
            LayoutRenderingElement::Label(label) => label.render(ctx),
            LayoutRenderingElement::Date(date) => date.render(ctx),
            LayoutRenderingElement::Names(names) => names.render(ctx),
            LayoutRenderingElement::Choose(choose) => choose.render(ctx),
            LayoutRenderingElement::Group(_group) => _group.render(ctx),
        }

        last_empty = ctx.writing.last_is_empty();
        if last_empty {
            ctx.discard_elem(pos);
        } else {
            ctx.commit_elem(pos, None, None);
        }
    }

    if let Some(loc) = loc {
        if last_empty {
            ctx.discard_elem(loc);
        } else {
            ctx.commit_elem(loc, None, None);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchConditionPos {
    Disambiguate,
    IsNumeric,
    IsUncertainDate,
    Locator,
    Position,
    Type,
    Variable,
}

impl Iterator for BranchConditionPos {
    type Item = Self;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Disambiguate => {
                *self = Self::IsNumeric;
                Some(Self::Disambiguate)
            }
            Self::IsNumeric => {
                *self = Self::IsUncertainDate;
                Some(Self::IsNumeric)
            }
            Self::IsUncertainDate => {
                *self = Self::Locator;
                Some(Self::IsUncertainDate)
            }
            Self::Locator => {
                *self = Self::Position;
                Some(Self::Locator)
            }
            Self::Position => {
                *self = Self::Type;
                Some(Self::Position)
            }
            Self::Type => {
                *self = Self::Variable;
                Some(Self::Type)
            }
            Self::Variable => None,
        }
    }
}

struct BranchConditionIter<'a, 'b, T: EntryLike> {
    cond: &'a ChooseBranch,
    ctx: &'a mut Context<'b, T>,
    pos: BranchConditionPos,
    idx: usize,
}

impl<'a, 'b, T: EntryLike> BranchConditionIter<'a, 'b, T> {
    fn from_branch(cond: &'a ChooseBranch, ctx: &'a mut Context<'b, T>) -> Self {
        Self {
            cond,
            ctx,
            pos: BranchConditionPos::Disambiguate,
            idx: 0,
        }
    }

    fn next_case(&mut self) {
        self.pos.next();
        self.idx = 0;
    }
}

impl<'a, 'b, T: EntryLike> Iterator for BranchConditionIter<'a, 'b, T> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        match self.pos {
            BranchConditionPos::Disambiguate => {
                self.pos.next();
                if let Some(d) = self.cond.disambiguate {
                    Some(d == self.ctx.should_disambiguate())
                } else {
                    self.next()
                }
            }
            BranchConditionPos::IsNumeric => {
                if let Some(vars) = &self.cond.is_numeric {
                    if self.idx >= vars.len() {
                        self.next_case();
                        return self.next();
                    }

                    let var = vars[self.idx];
                    self.idx += 1;

                    Some(match var {
                        Variable::Standard(var) => self
                            .ctx
                            .resolve_standard_variable(LongShortForm::default(), var)
                            .map(|v| Numeric::from_str(&v.to_string()).is_ok())
                            .unwrap_or_default(),
                        Variable::Number(var) => matches!(
                            self.ctx.resolve_number_variable(var),
                            Some(MaybeTyped::Typed(_))
                        ),
                        _ => false,
                    })
                } else {
                    self.next_case();
                    self.next()
                }
            }
            BranchConditionPos::IsUncertainDate => {
                if let Some(vars) = &self.cond.is_uncertain_date {
                    if self.idx >= vars.len() {
                        self.next_case();
                        return self.next();
                    }

                    let var = vars[self.idx];
                    self.idx += 1;

                    Some(
                        self.ctx
                            .resolve_date_variable(var)
                            .map_or(false, |d| d.approximate),
                    )
                } else {
                    self.next_case();
                    self.next()
                }
            }
            BranchConditionPos::Locator => {
                if let Some(locs) = &self.cond.locator {
                    if self.idx >= locs.len() {
                        self.next_case();
                        return self.next();
                    }

                    let loc = locs[self.idx];
                    self.idx += 1;

                    Some(
                        self.ctx
                            .instance
                            .cite_props
                            .speculative
                            .locator
                            .map(|l| l.0)
                            .map_or(false, |l| l == loc),
                    )
                } else {
                    self.next_case();
                    self.next()
                }
            }
            BranchConditionPos::Position => {
                if let Some(pos) = &self.cond.position {
                    if self.idx >= pos.len() {
                        self.next_case();
                        return self.next();
                    }

                    let spec_pos = pos[self.idx];
                    self.idx += 1;

                    let props = &self.ctx.instance.cite_props;

                    Some(match spec_pos {
                        TestPosition::First => props.certain.is_first,
                        TestPosition::Subsequent => !props.certain.is_first,
                        TestPosition::Ibid => props.speculative.ibid == IbidState::Ibid,
                        TestPosition::IbidWithLocator => {
                            props.speculative.ibid.is_ibid_with_locator()
                        }
                        TestPosition::NearNote => props.certain.is_near_note,
                    })
                } else {
                    self.next_case();
                    self.next()
                }
            }
            BranchConditionPos::Type => {
                if let Some(kind) = &self.cond.type_ {
                    if self.idx >= kind.len() {
                        self.next_case();
                        return self.next();
                    }

                    let kind = kind[self.idx];
                    self.idx += 1;

                    Some(self.ctx.instance.entry.matches_entry_type(kind))
                } else {
                    self.next_case();
                    self.next()
                }
            }
            BranchConditionPos::Variable => {
                if let Some(vars) = &self.cond.variable {
                    if self.idx >= vars.len() {
                        return None;
                    }

                    let var = vars[self.idx];
                    self.idx += 1;

                    Some(match var {
                        Variable::Standard(s) => {
                            let val = self
                                .ctx
                                .resolve_standard_variable(LongShortForm::default(), s);
                            val.map_or(false, |s| {
                                !s.to_string().chars().all(char::is_whitespace)
                            })
                        }
                        Variable::Number(n) => {
                            let val = self.ctx.resolve_number_variable(n);
                            val.is_some()
                        }
                        Variable::Date(d) => self.ctx.resolve_date_variable(d).is_some(),
                        Variable::Name(n) => {
                            !self.ctx.resolve_name_variable(n).is_empty()
                        }
                    })
                } else {
                    None
                }
            }
        }
    }
}

impl RenderCsl for citationberg::Group {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        let info = ctx.writing.push_usage_info();
        let idx = ctx.push_elem(self.to_formatting());
        let affixes = self.to_affixes();

        let affix_loc = ctx.apply_prefix(&affixes);

        render_with_delimiter(&self.children, self.delimiter.as_deref(), ctx);

        ctx.apply_suffix(&affixes, affix_loc);

        let info = ctx.writing.pop_usage_info(info);
        if info.has_vars
            && (!info.has_non_empty_vars
                && !info.has_used_macros
                && !info.has_non_empty_group)
        {
            ctx.discard_elem(idx);
        } else {
            ctx.commit_elem(idx, self.display, None);
            ctx.writing.printed_non_empty_group()
        }
    }
}

impl RenderCsl for citationberg::LayoutRenderingElement {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        match self {
            citationberg::LayoutRenderingElement::Text(text) => text.render(ctx),
            citationberg::LayoutRenderingElement::Number(num) => num.render(ctx),
            citationberg::LayoutRenderingElement::Label(label) => label.render(ctx),
            citationberg::LayoutRenderingElement::Date(date) => date.render(ctx),
            citationberg::LayoutRenderingElement::Names(names) => names.render(ctx),
            citationberg::LayoutRenderingElement::Choose(choose) => choose.render(ctx),
            citationberg::LayoutRenderingElement::Group(group) => group.render(ctx),
        }
    }
}

impl RenderCsl for citationberg::Layout {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        let fidx = ctx.push_format(self.to_formatting());
        for e in &self.elements {
            e.render(ctx);
        }
        ctx.pop_format(fidx);
    }
}

impl RenderCsl for citationberg::RenderingElement {
    fn render<T: EntryLike>(&self, ctx: &mut Context<T>) {
        match self {
            citationberg::RenderingElement::Layout(l) => l.render(ctx),
            citationberg::RenderingElement::Other(o) => o.render(ctx),
        }
    }
}

impl From<TextCase> for Case {
    fn from(case: TextCase) -> Self {
        match case {
            TextCase::Uppercase => Case::Uppercase,
            TextCase::Lowercase => Case::Lowercase,
            TextCase::TitleCase => Case::Title(TitleCase::default()),
            TextCase::SentenceCase => Case::Sentence(SentenceCase::default()),
            TextCase::CapitalizeFirst => Case::FirstUpper,
            TextCase::CapitalizeAll => Case::AllUpper,
        }
    }
}
