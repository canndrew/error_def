#![feature(plugin_registrar)]
#![feature(collections)]
#![feature(rustc_private)]

extern crate syntax;
extern crate rustc;

use syntax::codemap::{Span, spanned, DUMMY_SP, dummy_spanned, Spanned};
use syntax::ast::{self, TokenTree, EnumDef};
use syntax::ast::TokenTree::{TtToken, TtDelimited};
use syntax::ext::base::{ExtCtxt, MacResult, DummyResult, SyntaxExtension, MacEager};
use syntax::ext::build::AstBuilder;
use syntax::parse::token::{self, IdentStyle, intern, Token, Lit, get_name, DelimToken,
                           special_idents};
use syntax::parse;
use syntax::parse::attr::ParserAttr;
use syntax::util::small_vector::SmallVector;
use syntax::ast::{Variant_, Visibility, VariantKind, Variant, Attribute_, AttrStyle,
                  StrStyle, Lit_, MetaItem_, StructField, StructDef, Name, Unsafety, ImplPolarity,
                  TraitRef, Ty, Ty_, ImplItem, MethodSig, FnDecl, MutTy, Mutability, FunctionRetTy,
                  ExplicitSelf_, Block, Expr, Expr_, Arm, Pat, Pat_, MatchSource,
                  DUMMY_NODE_ID, BlockCheckMode, ImplItem_, Item, Item_, Path, PathSegment,
                  PathParameters, Arg, BindingMode, AngleBracketedParameterData, Delimited, Stmt_,
                  Mac_, FieldPat, StructFieldKind, Field};
use syntax::abi::Abi;
use syntax::ptr::P;
use syntax::attr::{mk_sugared_doc_attr, mk_attr_id};
use syntax::ext::quote::rt::ToTokens;
use syntax::ast_util;
use syntax::owned_slice::OwnedSlice;

use rustc::plugin::Registry;

use std::rc::Rc;

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

fn expand_error_def<'c>(cx: &mut ExtCtxt, sp: Span, type_name: ast::Ident, tokens: Vec<TokenTree>) -> Box<MacResult + 'c> {
  /*
  let foo = format!("token tree: {:?}", tokens);
  cx.span_err(sp, &foo[..]);
  return DummyResult::any(sp);
  */

  let mut parser = parse::tts_to_parser(cx.parse_sess(), tokens, cx.cfg());

  let mut items: Vec<P<ast::Item>> = Vec::new();
  let mut variants: Vec<VariantDef> = Vec::new();

  let syn_context = type_name.ctxt;

  loop {
    let var_lo = parser.span.lo;
    let variant_name = match parser.bump_and_get() {
      Ok(Token::Eof)                             => break,
      Ok(Token::Ident(ident, IdentStyle::Plain)) => ident,
      _ => {
        let _ = parser.fatal("Expected variant name");
        return DummyResult::any(sp);
      },
    };
    let var_hi = parser.span.hi;

    let (from_idx, members): (Option<usize>, Vec<StructField>) = match parser.bump_and_get() {
      Ok(Token::FatArrow) => (None, Vec::new()),
      Ok(Token::OpenDelim(DelimToken::Brace)) => {
        let mut members: Vec<StructField> = Vec::new();
        let mut from_memb_idx: Option<usize> = None;
        loop {
          let mut attrs = parser.parse_outer_attributes();
          let mut from_attr_idx: Option<usize> = None;
          for (i, attr) in attrs.iter().enumerate() {
            if let MetaItem_::MetaWord(ref attr_name) = attr.node.value.node {
              if *attr_name == "from" {
                match from_attr_idx {
                  Some(_) => {
                    let _ = parser.fatal("Field marked #[from] twice");
                    return DummyResult::any(sp);
                  },
                  None => from_attr_idx = Some(i),
                };
              };
            };
          };
          match from_attr_idx {
            Some(i) => {
              attrs.swap_remove(i);
              match from_memb_idx {
                Some(_) => {
                  let _ = parser.fatal("Multiple fields marked #[from]");
                  return DummyResult::any(sp);
                },
                None  => from_memb_idx = Some(members.len()),
              };
            },
            None    => (),
          };

          let sf = match parser.parse_single_struct_field(Visibility::Public, attrs) {
            Ok(sf)  => sf,
            Err(_)  => {
              let _ = parser.fatal("Expected struct field");
              return DummyResult::any(sp);
            },
          };
          match sf.node.kind {
            StructFieldKind::UnnamedField(_)  => {
              let _ = parser.fatal("Expected a named field");
              return DummyResult::any(sp);
            },
            _ => (),
          }
          members.push(sf);
          if parser.token == Token::CloseDelim(DelimToken::Brace) {
            let _ = parser.bump();
            break;
          }
        };
        
        match parser.bump_and_get() {
          Ok(Token::FatArrow) => (),
          _ => {
            let _ = parser.fatal("Expected =>");
            return DummyResult::any(sp);
          },
        };

        (from_memb_idx, members)
      },
      _ => {
        let _ = parser.fatal("Expected => or struct definition");
        return DummyResult::any(sp);
      },
    };

    let short_desc = match parser.bump_and_get() {
      Ok(Token::Literal(Lit::Str_(sd), None)) => sd,
      _ => {
        let _ = parser.fatal("Expected a string literal");
        return DummyResult::any(sp);
      },
    };

    let long_desc = if parser.token == Token::OpenDelim(DelimToken::Paren) {
      let _ = parser.bump();

      let format_str = match parser.bump_and_get() {
        Ok(Token::Literal(Lit::Str_(sd), None)) => sd,
        _ => {
          let _ = parser.fatal("Expected a format string");
          return DummyResult::any(sp);
        },
      };

      let mut format_args: Vec<P<Expr>> = Vec::new();
      loop {
        match parser.bump_and_get() {
          Ok(Token::Comma) => (),
          Ok(Token::CloseDelim(DelimToken::Paren)) => break,
          _ => {
            let _ = parser.fatal("Expected comma");
            return DummyResult::any(sp);
          },
        };
        format_args.push(parser.parse_expr());
      };

      Some(LongDescription {
        format_str: format_str,
        format_args: format_args,
      })
    }
    else {
      None
    };

    let comment_str = format!("/// {}.", short_desc);
    let comment = get_name(intern(&comment_str[..]));

    variants.push(VariantDef {
      variant: P(spanned(var_lo, var_hi, Variant_ {
        name:      variant_name,
        attrs:     vec![mk_sugared_doc_attr(mk_attr_id(), comment, var_lo, var_hi)],
        kind:      match members.len() {
          0 => VariantKind::TupleVariantKind(Vec::new()),
          _ => VariantKind::StructVariantKind(P(StructDef {
            fields:  members,
            ctor_id: None,
          })),
        },
        id:        DUMMY_NODE_ID,
        disr_expr: None,
        vis:       Visibility::Inherited,
      })),
      short_description: short_desc,
      from_idx: from_idx,
      long_description: long_desc,
    });

    match parser.bump_and_get() {
      Ok(Token::Comma)  => (),
      Ok(Token::Eof)    => (),
      _ => {
        let _ = parser.fatal("Expected comma");
        return DummyResult::any(sp);
      },
    }
  };

  let vars = variants.iter().map(|v| v.variant.clone()).collect();

  items.push(P(Item {
    ident: type_name,
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemEnum(
      EnumDef {
        variants: vars,
      },
      ast_util::empty_generics()
    ),
    vis:   Visibility::Public,
    span:  DUMMY_SP,
  }));

  let str_type = P(Ty {
    id: DUMMY_NODE_ID,
    node: Ty_::TyRptr(None, MutTy {
      ty: P(Ty {
        id: DUMMY_NODE_ID,
        node: Ty_::TyPath(None, path_from_segments(false, &[ast::Ident::new(intern("str"))])),
        span: DUMMY_SP,
      }),
      mutbl: Mutability::MutImmutable,
    }),
    span: DUMMY_SP,
  });

  let unused_attr = dummy_spanned(Attribute_ {
    id: mk_attr_id(),
    style: AttrStyle::AttrOuter,
    value: P(dummy_spanned(MetaItem_::MetaList(
      get_name(intern("allow")),
      vec![P(dummy_spanned(MetaItem_::MetaWord(get_name(intern("unused_variables")))))]
    ))),
    is_sugared_doc: false,
  });

  let fmt_meth_sig = MethodSig {
    unsafety: Unsafety::Normal,
    abi:      Abi::Rust,
    decl:     P(FnDecl {
      inputs: vec![
        Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_),
        Arg {
          ty: P(Ty {
            id: DUMMY_NODE_ID,
            node: Ty_::TyRptr(None, MutTy {
              ty: P(Ty {
                id:   DUMMY_NODE_ID,
                node: Ty_::TyPath(None, path_from_segments(true, &[
                  ast::Ident::new(intern("std")),
                  ast::Ident::new(intern("fmt")),
                  ast::Ident::new(intern("Formatter")),
                ])),
                span: DUMMY_SP,
              }),
              mutbl: Mutability::MutMutable,
            }),
            span: DUMMY_SP,
          }),
          pat: P(Pat {
            id:   DUMMY_NODE_ID,
            node: Pat_::PatIdent(BindingMode::BindByValue(Mutability::MutImmutable), dummy_spanned(ast::Ident::new(intern("f"))), None),
            span: DUMMY_SP,
          }),
          id: DUMMY_NODE_ID,
        },
      ],
      output: FunctionRetTy::Return(P(Ty {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Ty_::TyPath(None, Path {
          span: DUMMY_SP,
          global: true,
          segments: vec![
            PathSegment {
              identifier: ast::Ident::new(intern("std")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::new(intern("result")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::new(intern("Result")),
              parameters: PathParameters::AngleBracketedParameters(AngleBracketedParameterData {
                lifetimes: Vec::new(),
                types:     OwnedSlice::from_vec(vec![
                  P(Ty {
                    id:   DUMMY_NODE_ID,
                    span: DUMMY_SP,
                    node: Ty_::TyTup(Vec::new()),
                  }),
                  P(Ty {
                    id:   DUMMY_NODE_ID,
                    span: DUMMY_SP,
                    node: Ty_::TyPath(None, path_from_segments(true, &[
                      ast::Ident::new(intern("std")),
                      ast::Ident::new(intern("fmt")),
                      ast::Ident::new(intern("Error")),
                    ])),
                  })
                ]),
                bindings:  OwnedSlice::empty(),
              }),
            },
          ],
        }),
      })),
      variadic: false,
    }),
    generics: ast_util::empty_generics(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::new(intern("what_is_this")))),
  };

  let debug_fmt_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: Expr_::ExprBlock(P(Block {
      stmts: vec![{
        match v.variant.node.kind {
          VariantKind::StructVariantKind(ref sd) => {
            let mut ss = format!("{} {{{{", v.variant.node.name);
            let fields = &sd.fields;
            let mut first = true;
            for f in fields.iter() {
              if !first {
                ss.push(',');
              };
              first = false;
              let field_name = match f.node.kind {
                StructFieldKind::NamedField(ident, _) => ident,
                _                                     => unreachable!(),
              };
              ss.push_str(&format!(" {}: {{:?}}", field_name)[..]);
            }
            ss.push_str(" }} /* {} */");
            P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Expr_::ExprMac(dummy_spanned(Mac_::MacInvocTT(path_from_segments(false, &[ast::Ident::new(intern("try"))]), vec![
                TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("write")), IdentStyle::Plain)),
                TtToken(DUMMY_SP, Token::Not),
                TtDelimited(DUMMY_SP, Rc::new(Delimited {
                  delim: DelimToken::Paren,
                  open_span:  DUMMY_SP,
                  close_span: DUMMY_SP,
                  tts: {
                    let mut tts = vec![
                      TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("f")), IdentStyle::Plain)),
                      TtToken(DUMMY_SP, Token::Comma),
                      TtToken(DUMMY_SP, Token::Literal(Lit::Str_(intern(&ss[..])), None)),
                    ];
                    for f in fields.iter() {
                      tts.push(TtToken(DUMMY_SP, Token::Comma));
                      let field_name = match f.node.kind {
                        StructFieldKind::NamedField(ident, _)  => ident,
                        _                                      => unreachable!(),
                      };
                      tts.push(TtToken(DUMMY_SP, Token::Ident(field_name, IdentStyle::Plain)));
                    };
                    tts.push(TtToken(DUMMY_SP, Token::Comma));
                    tts.push(TtToken(DUMMY_SP, Token::Ident(special_idents::self_, IdentStyle::Plain)));
                    tts
                  },
                })),
              ], syn_context))),
            }), DUMMY_NODE_ID)))
          },
          VariantKind::TupleVariantKind(_) => {
            let ss = format!("{} /* {{}} */", v.variant.node.name);
            P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Expr_::ExprMac(dummy_spanned(Mac_::MacInvocTT(path_from_segments(false, &[ast::Ident::new(intern("try"))]), vec![
                TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("write")), IdentStyle::Plain)),
                TtToken(DUMMY_SP, Token::Not),
                TtDelimited(DUMMY_SP, Rc::new(Delimited {
                  delim: DelimToken::Paren,
                  open_span:  DUMMY_SP,
                  close_span: DUMMY_SP,
                  tts: vec![
                    TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("f")), IdentStyle::Plain)),
                    TtToken(DUMMY_SP, Token::Comma),
                    TtToken(DUMMY_SP, Token::Literal(Lit::Str_(intern(&ss[..])), None)),
                    TtToken(DUMMY_SP, Token::Comma),
                    TtToken(DUMMY_SP, Token::Ident(special_idents::self_, IdentStyle::Plain)),
                  ],
                })),
              ], syn_context))),
            }), DUMMY_NODE_ID)))
          }
        }
      }],
      expr: Some(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCall(
          P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::new(intern("Ok"))])),
          }),
          vec![
            P(Expr {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Expr_::ExprTup(Vec::new()),
            }),
          ]
        ),
      })),
      id:    DUMMY_NODE_ID,
      span:  DUMMY_SP,
      rules: BlockCheckMode::DefaultBlock,
    })),
  }));

  let debug_fmt_impl = P(ImplItem {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    ident: ast::Ident::new(intern("fmt")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItem_::MethodImplItem(fmt_meth_sig.clone(), debug_fmt_block),
  });

  let display_fmt_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: Expr_::ExprBlock(P(Block {
      stmts: {
        let mut try_writes = vec![
          P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_::MacInvocTT(path_from_segments(false, &[ast::Ident::new(intern("try"))]), vec![
              TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("write")), IdentStyle::Plain)),
              TtToken(DUMMY_SP, Token::Not),
              TtDelimited(DUMMY_SP, Rc::new(Delimited {
                delim: DelimToken::Paren,
                open_span:  DUMMY_SP,
                close_span: DUMMY_SP,
                tts: vec![
                  TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("f")), IdentStyle::Plain)),
                  TtToken(DUMMY_SP, Token::Comma),
                  TtToken(DUMMY_SP, Token::Literal(Lit::Str_(v.short_description), None)),
                ],
              })),
            ], syn_context))),
          }), DUMMY_NODE_ID))),
          P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_::MacInvocTT(path_from_segments(false, &[ast::Ident::new(intern("try"))]), vec![
              TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("write")), IdentStyle::Plain)),
              TtToken(DUMMY_SP, Token::Not),
              TtDelimited(DUMMY_SP, Rc::new(Delimited {
                delim: DelimToken::Paren,
                open_span:  DUMMY_SP,
                close_span: DUMMY_SP,
                tts: vec![
                  TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("f")), IdentStyle::Plain)),
                  TtToken(DUMMY_SP, Token::Comma),
                  TtToken(DUMMY_SP, Token::Literal(Lit::Str_(intern(". ")), None)),
                ],
              })),
            ], syn_context))),
          }), DUMMY_NODE_ID))),
        ];
        if let Some(ref long_desc) = v.long_description {
          try_writes.push(P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_::MacInvocTT(path_from_segments(false, &[ast::Ident::new(intern("try"))]), vec![
              TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("write")), IdentStyle::Plain)),
              TtToken(DUMMY_SP, Token::Not),
              TtDelimited(DUMMY_SP, Rc::new(Delimited {
                delim: DelimToken::Paren,
                open_span:  DUMMY_SP,
                close_span: DUMMY_SP,
                tts: {
                  let mut write_args = vec![
                    TtToken(DUMMY_SP, Token::Ident(ast::Ident::new(intern("f")), IdentStyle::Plain)),
                    TtToken(DUMMY_SP, Token::Comma),
                    TtToken(DUMMY_SP, Token::Literal(Lit::Str_(long_desc.format_str), None)),
                  ];
                  for fa in long_desc.format_args.iter() {
                    write_args.push(TtToken(DUMMY_SP, Token::Comma));
                    let tt = fa.to_tokens(cx);
                    write_args.push_all(&tt[..]);
                  };
                  write_args
                },
              })),
            ], syn_context))),
          }), DUMMY_NODE_ID))));
        };
        try_writes
      },
      expr: Some(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCall(
          P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::new(intern("Ok"))])),
          }),
          vec![
            P(Expr {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Expr_::ExprTup(Vec::new()),
            }),
          ]
        ),
      })),
      id:    DUMMY_NODE_ID,
      span:  DUMMY_SP,
      rules: BlockCheckMode::DefaultBlock,
    })),
  }));

  let display_fmt_impl = P(ImplItem {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    ident: ast::Ident::new(intern("fmt")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItem_::MethodImplItem(fmt_meth_sig, display_fmt_block),
  });

  let description_meth_sig = MethodSig {
    unsafety: Unsafety::Normal,
    abi:      Abi::Rust,
    decl:     P(FnDecl {
      inputs:   vec![Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_)],
      output:   FunctionRetTy::Return(str_type),
      variadic: false,
    }),
    generics: ast_util::empty_generics(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::new(intern("what_is_this")))),
  };

  let description_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: Expr_::ExprLit(P(dummy_spanned(Lit_::LitStr(get_name(v.short_description), StrStyle::CookedStr)))),
  }));

  let description_impl = P(ImplItem {
    id:    DUMMY_NODE_ID,
    span:  DUMMY_SP,
    ident: ast::Ident::new(intern("description")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItem_::MethodImplItem(description_meth_sig, description_block),
  });

  let ref_error_ty = P(Ty {
    node: Ty_::TyRptr(None, MutTy {
      ty: P(Ty {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Ty_::TyPath(None, path_from_segments(true, &[
          ast::Ident::new(intern("std")),
          ast::Ident::new(intern("error")),
          ast::Ident::new(intern("Error")),
        ])),
      }),
      mutbl: Mutability::MutImmutable,
    }),
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
  });

  let cause_meth_sig = MethodSig {
    unsafety: Unsafety::Normal,
    abi:      Abi::Rust,
    decl:     P(FnDecl {
      inputs:   vec![Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_)],
      output:   FunctionRetTy::Return(P(Ty {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Ty_::TyPath(None, Path {
          span:   DUMMY_SP,
          global: true,
          segments: vec![
            PathSegment {
              identifier: ast::Ident::new(intern("std")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::new(intern("option")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::new(intern("Option")),
              parameters: PathParameters::AngleBracketedParameters(AngleBracketedParameterData {
                lifetimes: Vec::new(),
                types:     OwnedSlice::from_vec(vec![ref_error_ty.clone()]),
                bindings:  OwnedSlice::empty(),
              }),
            },
          ],
        }),
      })),
      variadic: false,
    }),
    generics: ast_util::empty_generics(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::new(intern("what_is_this")))),
  };

  let cause_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: match v.from_idx {
      Some(i) => Expr_::ExprCall(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprPath(None, path_from_segments(true, &[
          ast::Ident::new(intern("std")),
          ast::Ident::new(intern("option")),
          ast::Ident::new(intern("Option")),
          ast::Ident::new(intern("Some")),
        ])),
      }), vec![P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCast(P(Expr {
          id:   DUMMY_NODE_ID,
          span: DUMMY_SP,
          node: Expr_::ExprPath(None, path_from_segments(false, &[
            match v.variant.node.kind {
              VariantKind::StructVariantKind(ref sd) => match sd.fields[i].node.kind {
                StructFieldKind::NamedField(ident, _) => ident,
                StructFieldKind::UnnamedField(_)      => unreachable!(),
              },
              VariantKind::TupleVariantKind(_)       => unreachable!(),
            },
          ])),
        }), ref_error_ty.clone()),
      })]),
      None    => Expr_::ExprPath(None, path_from_segments(true, &[
        ast::Ident::new(intern("std")),
        ast::Ident::new(intern("option")),
        ast::Ident::new(intern("Option")),
        ast::Ident::new(intern("None")),
      ])),
    },
  }));

  let cause_impl = P(ImplItem {
    id:    DUMMY_NODE_ID,
    span:  DUMMY_SP,
    ident: ast::Ident::new(intern("cause")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItem_::MethodImplItem(cause_meth_sig, cause_block),
  });

  items.push(P(Item {
    ident: ast::Ident::new(intern("whats_this_then")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, ast_util::empty_generics(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::new(intern("std")),
        ast::Ident::new(intern("fmt")),
        ast::Ident::new(intern("Debug"))
      ]),
      ref_id: DUMMY_NODE_ID,
    }), P(Ty {
      id:   DUMMY_NODE_ID,
      node: Ty_::TyPath(None, path_from_segments(false, &[type_name])),
      span: DUMMY_SP,
    }), vec![
      debug_fmt_impl,
    ]),
    vis:  Visibility::Inherited,
    span: DUMMY_SP,
  }));

  items.push(P(Item {
    ident: ast::Ident::new(intern("whats_this_then")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, ast_util::empty_generics(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::new(intern("std")),
        ast::Ident::new(intern("fmt")),
        ast::Ident::new(intern("Display"))
      ]),
      ref_id: DUMMY_NODE_ID,
    }), P(Ty {
      id:   DUMMY_NODE_ID,
      node: Ty_::TyPath(None, path_from_segments(false, &[type_name])),
      span: DUMMY_SP,
    }), vec![
      display_fmt_impl,
    ]),
    vis:  Visibility::Inherited,
    span: DUMMY_SP,
  }));

  items.push(P(Item {
    ident: ast::Ident::new(intern("seriously_what_should_this_be")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, ast_util::empty_generics(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::new(intern("std")),
        ast::Ident::new(intern("error")),
        ast::Ident::new(intern("Error"))
      ]),
      ref_id: DUMMY_NODE_ID,
    }), P(Ty {
      id:   DUMMY_NODE_ID,
      node: Ty_::TyPath(None, path_from_segments(false, &[type_name])),
      span: DUMMY_SP,
    }), vec![
      description_impl,
      cause_impl,
    ]),
    vis:  Visibility::Inherited,
    span: DUMMY_SP,
  }));

  for v in variants {
    if let VariantKind::StructVariantKind(ref sd) = v.variant.node.kind {
      if sd.fields.len() == 1 && v.from_idx == Some(0) {
        let field = &sd.fields[0].node;
        let from_meth_sig = MethodSig {
          unsafety:      Unsafety::Normal,
          abi:           Abi::Rust,
          decl:          P(FnDecl {
            inputs: vec![Arg {
              ty:  field.ty.clone(),
              pat: P(Pat {
                node: Pat_::PatIdent(BindingMode::BindByValue(Mutability::MutImmutable), dummy_spanned(ast::Ident::new(intern("e"))), None),
                id:   DUMMY_NODE_ID,
                span: DUMMY_SP,
              }),
              id:  DUMMY_NODE_ID,
            }],
            output: FunctionRetTy::Return(P(Ty {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Ty_::TyPath(None, path_from_segments(false, &[type_name])),
            })),
            variadic: false,
          }),
          generics:      ast_util::empty_generics(),
          explicit_self: dummy_spanned(ExplicitSelf_::SelfStatic),
        };
        let from_meth_block = P(Block {
          stmts: Vec::new(),
          expr:  Some(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprStruct(
              path_from_segments(false, &[type_name, v.variant.node.name]),
              vec![Field {
                ident: match field.kind {
                  StructFieldKind::NamedField(ident, _)  => dummy_spanned(ident),
                  StructFieldKind::UnnamedField(_)       => panic!("not possible"),
                },
                expr: P(Expr {
                  node: Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::new(intern("e"))])),
                  id:   DUMMY_NODE_ID,
                  span: DUMMY_SP,
                }),
                span: DUMMY_SP,
              }],
              None,
            ),
          })),
          id:    DUMMY_NODE_ID,
          rules: BlockCheckMode::DefaultBlock,
          span:  DUMMY_SP,
        });
        let from_meth_impl = P(ImplItem {
          id:    DUMMY_NODE_ID,
          span:  DUMMY_SP,
          ident: ast::Ident::new(intern("from")),
          vis:   Visibility::Inherited,
          attrs: Vec::new(),
          node:  ImplItem_::MethodImplItem(from_meth_sig, from_meth_block),
        });

        items.push(P(Item {
          ident: ast::Ident::new(intern("zoomzoom")),
          attrs: Vec::new(),
          id:    DUMMY_NODE_ID,
          node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, ast_util::empty_generics(), Some(TraitRef {
            path:   Path {
              span:   DUMMY_SP,
              global: true,
              segments: vec![
                PathSegment {
                  identifier: ast::Ident::new(intern("std")),
                  parameters: PathParameters::none(),
                },
                PathSegment {
                  identifier: ast::Ident::new(intern("convert")),
                  parameters: PathParameters::none(),
                },
                PathSegment {
                  identifier: ast::Ident::new(intern("From")),
                  parameters: PathParameters::AngleBracketedParameters(AngleBracketedParameterData {
                    lifetimes: Vec::new(),
                    types:     OwnedSlice::from_vec(vec![field.ty.clone()]),
                    bindings:  OwnedSlice::empty(),
                  }),
                },
              ],
            },
            ref_id: DUMMY_NODE_ID,
          }), P(Ty {
            id:   DUMMY_NODE_ID,
            node: Ty_::TyPath(None, path_from_segments(false, &[type_name])),
            span: DUMMY_SP,
          }), vec![
            from_meth_impl,
          ]),
          vis:  Visibility::Inherited,
          span: DUMMY_SP,
        }));
      }
    }
  }

  MacEager::items((SmallVector::many(items)))
}

#[plugin_registrar]
pub fn plugin_registrar(reg: &mut Registry) {
  reg.register_syntax_extension(token::intern("error_def"), SyntaxExtension::IdentTT(Box::new(expand_error_def), None, false));
}

fn path_from_segments(global: bool, segments: &[ast::Ident]) -> Path {
  Path {
    span:   DUMMY_SP,
    global: global,
    segments: segments.iter().map(|i| PathSegment {
      identifier: *i,
      parameters: PathParameters::none()
    }).collect(),
  }
}

fn mk_match_block<F>(variants: &Vec<VariantDef>, type_name: ast::Ident, func: F) -> P<Block>
    where F: Fn(&VariantDef) -> P<Expr>
{
  let expr_self = P(Expr {
    id: DUMMY_NODE_ID,
    node: Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::new(intern("self"))])),
    span: DUMMY_SP,
  });

  P(Block {
    stmts: Vec::new(),
    expr:  Some(P(Expr {
      id:   DUMMY_NODE_ID,
      node: Expr_::ExprMatch(
        expr_self,
        {
          let mut arms: Vec<Arm> = Vec::new();
          for v in variants {
            arms.push(Arm {
              attrs: Vec::new(),
              pats:  vec![P(Pat {
                id:   DUMMY_NODE_ID,
                span: DUMMY_SP,
                node: Pat_::PatRegion(
                  P(Pat {
                    id: DUMMY_NODE_ID,
                    node: match v.variant.node.kind {
                      VariantKind::StructVariantKind(ref sd)  => Pat_::PatStruct(
                        path_from_segments(false, &[type_name, v.variant.node.name]),
                        {
                          let mut pat_fields: Vec<Spanned<FieldPat>> = Vec::new();
                          for field in sd.fields.iter() {
                            pat_fields.push(dummy_spanned(FieldPat {
                              ident: match field.node.kind {
                                StructFieldKind::NamedField(ident, _)  => ident,
                                StructFieldKind::UnnamedField(_)       => panic!("not possible"),
                              },
                              pat:  P(Pat {
                                id:   DUMMY_NODE_ID,
                                span: DUMMY_SP,
                                node: Pat_::PatIdent(BindingMode::BindByRef(Mutability::MutImmutable), dummy_spanned(match field.node.kind {
                                  StructFieldKind::NamedField(ident, _)  => ident,
                                  StructFieldKind::UnnamedField(_)       => panic!("not possible"),
                                }), None),
                              }),
                              is_shorthand: true,
                            }))
                          };
                          pat_fields
                        },
                        false,
                      ),
                      VariantKind::TupleVariantKind(_) => Pat_::PatEnum(
                        path_from_segments(false, &[type_name, v.variant.node.name]),
                        None,
                      ),
                    },
                    span: DUMMY_SP,
                  }),
                  Mutability::MutImmutable,
                ),
              })],
              guard: None,
              body: func(v),
            });
          };
          arms
        },
        MatchSource::Normal,
      ),
      span: DUMMY_SP,
    })),
    id:    DUMMY_NODE_ID,
    rules: BlockCheckMode::DefaultBlock,
    span:  DUMMY_SP,
  })
}

