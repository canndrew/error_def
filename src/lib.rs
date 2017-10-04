#![feature(quote)]
#![feature(plugin_registrar)]
#![feature(rustc_private)]
#![allow(unused_imports)]

extern crate syntax;
extern crate rustc_plugin;

use syntax::codemap::{Span, DUMMY_SP, dummy_spanned};
use syntax::tokenstream::TokenTree;
use syntax::ext::base::{ExtCtxt, MacResult, DummyResult, SyntaxExtension, MacEager};
use syntax::parse::token::{Token, DelimToken};
use syntax::symbol::Symbol;
use syntax::parse;
use syntax::util::small_vector::SmallVector;
use syntax::ast::{self, Variant_, Visibility, VariantData, Variant, LitKind, StructField, Name,
                  Expr, DUMMY_NODE_ID};
use syntax::ptr::P;
use syntax::attr::{mk_sugared_doc_attr, mk_attr_id};
use syntax::ext::quote::rt::ToTokens;

use rustc_plugin::Registry;

struct LongDescription {
    format_str: Name,
    format_args: Vec<P<Expr>>
}

struct VariantDef {
    variant: P<Variant>,
    short_description: Name,
    from_idx: Option<usize>,
    long_description: Option<LongDescription>,
}

fn expand_error_def<'c>(
    cx: &'c mut ExtCtxt,
    sp: Span,
    type_name: ast::Ident,
    tokens: Vec<TokenTree>
) -> Box<MacResult + 'c> {
    let mut parser = parse::stream_to_parser(cx.parse_sess(), tokens.into_iter().collect());

    let mut items: Vec<P<ast::Item>> = Vec::new();
    let mut variants: Vec<VariantDef> = Vec::new();

    // Parse the token tree and populate our list of variants.
    loop {
        let variant_name = if parser.check(&Token::Eof) {
            break
        } else {
            match parser.parse_ident() {
                Ok(ident) => ident,
                Err(mut e) => {
                    e.emit();
                    return DummyResult::any(sp);
                },
            }
        };

        let (from_idx, members): (Option<usize>, Option<Vec<StructField>>) = if parser.eat(&Token::FatArrow) {
            // It's a unit-like variant. (ie. not a struct variant)
            (None, None)
        } else if parser.eat(&Token::OpenDelim(DelimToken::Brace)) {
            // It's a struct variant
            let mut members: Vec<StructField> = Vec::new();
            let mut from_memb_idx: Option<usize> = None;

            // Parse the list of struct members.
            loop {
                
                // Parse the list of attributes on this struct member
                let mut attrs = match parser.parse_outer_attributes() {
                    Ok(attrs) => attrs,
                    Err(mut e) => {
                        e.emit();
                        return DummyResult::any(sp);
                    },
                };

                // Find whether this member is marked #[from]. And if it is, find the index of the
                // #[from] attribute so we can remove it.

                let mut from_attr_idx: Option<(usize, Span)> = None;
                for (i, attr) in attrs.iter().enumerate() {
                    if attr.path == "from" {
                        match from_attr_idx {
                            Some(_) => {
                                cx.span_err(attr.span, "Field marked #[from] twice");
                                return DummyResult::any(sp);
                            },
                            None => from_attr_idx = Some((i, attr.span)),
                        };
                    };
                };
                match from_attr_idx {
                    // This member is marked #[from]. Record this.
                    Some((i, attr_span)) => {
                        attrs.swap_remove(i);
                        match from_memb_idx {
                            Some(_) => {
                                cx.span_err(attr_span, "Multiple fields marked #[from]");
                                return DummyResult::any(sp);
                            },
                            None  => from_memb_idx = Some(members.len()),
                        };
                    },
                    None    => (),
                };

                // Parse the name and type of the member.
                let sf = match parser.parse_single_struct_field(DUMMY_SP,
                                                                Visibility::Inherited,
                                                                attrs) {
                    Ok(sf)  => sf,
                    Err(mut e)  => {
                        e.emit();
                        return DummyResult::any(sp);
                    },
                };
                if sf.ident.is_none() {
                    cx.span_err(sp, "Expected a named field");
                    return DummyResult::any(sp);
                }
                members.push(sf);
                if parser.token == Token::CloseDelim(DelimToken::Brace) {
                    let _ = parser.bump();
                    break;
                }

            };
      
            if let Err(mut e) = parser.expect(&Token::FatArrow) {
                e.emit();
                return DummyResult::any(sp);
            };

            (from_memb_idx, Some(members))
        } else {
            match parser.expect_one_of(&[Token::FatArrow, Token::OpenDelim(DelimToken::Brace)], &[]) {
                Ok(..) => unreachable!(),
                Err(mut e) => {
                    e.emit();
                    return DummyResult::any(sp);
                },
            }
        };

        // Parse the short description.
        let short_desc = match parser.parse_str() {
            Ok((sd, _)) => sd,
            Err(mut e) => {
                e.emit();
                return DummyResult::any(sp);
            },
        };

        // Parse the long description if it exists.
        let long_desc = if parser.token == Token::OpenDelim(DelimToken::Paren) {
            let _ = parser.bump();

            let format_str = match parser.parse_str() {
                Ok((fs, _)) => fs,
                Err(mut e) => {
                    e.emit();
                    return DummyResult::any(sp);
                },
            };

            let mut format_args: Vec<P<Expr>> = Vec::new();
            loop {
                if parser.eat(&Token::CloseDelim(DelimToken::Paren)) {
                    break
                } else if let Err(mut e) = parser.expect(&Token::Comma) {
                    e.emit();
                    return DummyResult::any(sp);
                };
                let ex = match parser.parse_expr() {
                    Ok(ex) => ex,
                    Err(mut e) => {
                        e.emit();
                        return DummyResult::any(sp);
                    },
                };
                format_args.push(ex);
            };

            Some(LongDescription {
                format_str: format_str,
                format_args: format_args,
            })
        } else {
            None
        };

        let comment_str = format!("/// {}.", short_desc);
        let comment = Symbol::intern(&comment_str[..]);

        // Build our variant definition out of the information we've parsed.
        variants.push(VariantDef {
            variant: P(dummy_spanned(Variant_ {
                name:      variant_name,
                attrs:     vec![mk_sugared_doc_attr(mk_attr_id(), comment, DUMMY_SP)],
                data:      match members {
                    None => VariantData::Unit(DUMMY_NODE_ID),
                    Some(members) => VariantData::Struct(members, DUMMY_NODE_ID),
                },
                disr_expr: None,
            })),
            short_description: short_desc,
            from_idx: from_idx,
            long_description: long_desc,
        }); 

        if let Err(mut e) = parser.expect_one_of(&[Token::Comma], &[Token::Eof]) {
            e.emit();
            return DummyResult::any(sp);
        }
    }

    // Helper wrappers for missing ToTokens impls

    struct StructFieldWrapper(StructField);
    impl ToTokens for StructFieldWrapper {
        fn to_tokens(&self, cx: &ExtCtxt) -> Vec<TokenTree> {
            let StructFieldWrapper(ref struct_field) = *self;
            let StructField {
                ref ty,
                ref ident,
                ref attrs,
                ..
            } = *struct_field;

            quote_tokens!(cx, $attrs $ident: $ty,)
        }
    }

    struct VariantWrapper(Variant);
    impl ToTokens for VariantWrapper {
        fn to_tokens(&self, cx: &ExtCtxt) -> Vec<TokenTree> {
            let VariantWrapper(ref variant) = *self;
            let Variant_ {
                ref attrs,
                ref name,
                ref data,
                ..
            } = variant.node;

            match *data {
                VariantData::Unit(..) => quote_tokens!(cx, $attrs $name,),
                VariantData::Struct(ref members, _) => {
                    let members_wrapped: Vec<_> = members.iter().map(|m| StructFieldWrapper(m.clone())).collect();
                    quote_tokens!(cx, $attrs $name {
                        $members_wrapped
                    },)
                },
                _ => unreachable!(),
            }
        }
    }


    // Add the enum

    let mut variants_wrapped = Vec::new();
    for v in &variants {
        let variant = VariantWrapper((*v.variant).clone());
        variants_wrapped.push(variant);
    }

    let the_enum = quote_item!(cx, pub enum $type_name {
        $variants_wrapped
    });

    items.push(the_enum.unwrap());

    // Add Debug impl

    let mut debug_impl_arms = Vec::new();
    for v in &variants {
        let VariantDef {
            ref variant,
            ..
        } = *v;
        let Variant_ {
            ref name,
            ref data,
            ..
        } = variant.node;

        let full_name = Symbol::intern(&format!("{}::{}", type_name, name));
        let full_name = dummy_spanned(ast::LitKind::Str(full_name, ast::StrStyle::Cooked));
        let debug_impl_arm = match *data {
            VariantData::Unit(..) => {
                quote_tokens!(cx, $type_name::$name => {
                    write!(f, $full_name)?;
                    write!(f, " /* {} */", self)?;
                })
            },
            VariantData::Struct(ref members, ..) => {
                let mut ms = Vec::new();
                let mut body = quote_tokens!(cx, f.debug_struct($full_name));
                for member in members {
                    let StructField {
                        ref ident,
                        ..
                    } = *member;
                    let ident = ident.as_ref().unwrap();
                    ms.extend(quote_tokens!(cx, ref $ident,));
                    let ident_lit = dummy_spanned(ast::LitKind::Str(ident.name, ast::StrStyle::Cooked));
                    body.extend(quote_tokens!(cx, .field($ident_lit, $ident)));
                }
                body.extend(quote_tokens!(cx, .finish()?));

                quote_tokens!(cx, $type_name::$name { $ms } => {
                    $body;
                    write!(f, " /* {} */", self)?;
                })
            },
            _ => unreachable!(),
        };
        debug_impl_arms.push(debug_impl_arm);
    }

    let debug_impl = quote_item!(cx, impl ::std::fmt::Debug for $type_name {
        fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
            match *self {
                $debug_impl_arms
            }
            Ok(())
        }
    });

    items.push(debug_impl.unwrap());

    // Add Display impl

    let mut display_impl_arms = Vec::new();
    for v in &variants {
        let VariantDef {
            ref variant,
            ref short_description,
            ref long_description,
            ..
        } = *v;
        let Variant_ {
            ref name,
            ref data,
            ..
        } = variant.node;
        
        let short = dummy_spanned(ast::LitKind::Str(short_description.clone(), ast::StrStyle::Cooked));
        let print_short = quote_stmt!(cx, write!(f, $short)?;); 
        let mut body = quote_tokens!(cx, $print_short);
        if let Some(LongDescription { ref format_str, ref format_args }) = *long_description {
            let long_fmt = format!(". {}", format_str);
            let long_fmt = Symbol::intern(&long_fmt[..]);
            let long = dummy_spanned(ast::LitKind::Str(long_fmt, ast::StrStyle::Cooked));
            let mut args = Vec::new();
            for arg in format_args {
                args.extend(quote_tokens!(cx, $arg,));
            }
            let print_long = quote_stmt!(cx, write!(f, $long, $args)?;);
            body.extend(quote_tokens!(cx, $print_long));
        }

        let display_impl_arm = match *data {
            VariantData::Unit(..) => {
                quote_tokens!(cx, $type_name::$name => {
                    $body
                })
            },
            VariantData::Struct(ref members, ..) => {
                let mut ms = Vec::new();
                for member in members {
                    let StructField {
                        ref ident,
                        ..
                    } = *member;
                    ms.extend(quote_tokens!(cx, ref $ident,));
                }
                quote_tokens!(cx, $type_name::$name { $ms } => {
                    $body
                })
            },
            _ => unreachable!(),
        };
        display_impl_arms.push(display_impl_arm);
    }

    let display_impl = quote_item!(cx, impl ::std::fmt::Display for $type_name {
        fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
            #[allow(unused)]
            match *self {
                $display_impl_arms
            }
            Ok(())
        }
    });

    items.push(display_impl.unwrap());

    // Add Error impl

    let mut description_impl_arms = Vec::new();
    for v in &variants {
        let VariantDef {
            ref variant,
            ref short_description,
            ..
        } = *v;
        let Variant_ {
            ref name,
            ref data,
            ..
        } = variant.node;
        
        let short = dummy_spanned(ast::LitKind::Str(short_description.clone(), ast::StrStyle::Cooked));

        let description_impl_arm = match *data {
            VariantData::Unit(..) => {
                quote_tokens!(cx, $type_name::$name => {
                    $short
                })
            },
            VariantData::Struct(..) => {
                quote_tokens!(cx, $type_name::$name { .. } => {
                    $short
                })
            },
            _ => unreachable!(),
        };
        description_impl_arms.push(description_impl_arm);
    }

    let mut cause_impl_arms = Vec::new();
    for v in &variants {
        let VariantDef {
            ref variant,
            ref from_idx,
            ..
        } = *v;
        let Variant_ {
            ref name,
            ref data,
            ..
        } = variant.node;

        let cause_impl_arm = match *data {
            VariantData::Unit(..) => {
                quote_tokens!(cx, $type_name::$name => None,)
            },
            VariantData::Struct(ref members, ..) => {
                let mut ms = Vec::new();
                for member in members {
                    let StructField {
                        ref ident,
                        ..
                    } = *member;
                    ms.extend(quote_tokens!(cx, ref $ident,));
                }
                let expr = match *from_idx {
                    None => quote_expr!(cx, None),
                    Some(idx) => {
                        let StructField {
                            ref ident,
                            ..
                        } = members[idx];
                        quote_expr!(cx, Some($ident))
                    },
                };
                quote_tokens!(cx, $type_name::$name { $ms } => $expr,)
            },
            _ => unreachable!(),
        };
        cause_impl_arms.push(cause_impl_arm);
    }

    let error_impl = quote_item!(cx, impl ::std::error::Error for $type_name {
        fn description(&self) -> &str {
            match *self {
                $description_impl_arms
            }
        }

        fn cause(&self) -> Option<&::std::error::Error> {
            #[allow(unused)]
            match *self {
                $cause_impl_arms
            }
        }
    });

    items.push(error_impl.unwrap());

    // Add `From` impls
    for v in &variants {
        let VariantDef {
            ref variant,
            ref from_idx,
            ..
        } = *v;
        let Variant_ {
            ref name,
            ref data,
            ..
        } = variant.node;

        if let Some(idx) = *from_idx {
            let members = match *data {
                VariantData::Struct(ref members, ..) => members,
                _ => unreachable!(),
            };
            if members.len() != 1 {
                continue;
            }
            let StructField {
                ref ty,
                ref ident,
                ..
            } = members[idx];
            let from_impl = quote_item!(cx, impl ::std::convert::From<$ty> for $type_name {
                fn from(val: $ty) -> $type_name {
                    $type_name::$name { $ident: val }
                }
            });
            items.push(from_impl.unwrap());
        }
    }

    MacEager::items((SmallVector::many(items)))
}

#[plugin_registrar]
pub fn plugin_registrar(reg: &mut Registry) {
    reg.register_syntax_extension(
        Symbol::intern("error_def"),
        SyntaxExtension::IdentTT(Box::new(expand_error_def), None, false)
    );
}

