#![feature(plugin_registrar)]
#![feature(rustc_private)]

extern crate syntax;
extern crate rustc;
extern crate rustc_plugin;
extern crate rustc_front;

use syntax::codemap::{Span, spanned, DUMMY_SP, dummy_spanned, Spanned};
use syntax::ast::{self, TokenTree, EnumDef};
use syntax::ext::base::{ExtCtxt, MacResult, DummyResult, SyntaxExtension, MacEager};
use syntax::ext::build::AstBuilder;
use syntax::parse::token::{self, IdentStyle, intern, Token, Lit, InternedString, DelimToken,
                           special_idents};
use syntax::parse;
use syntax::util::small_vector::SmallVector;
use syntax::ast::{Variant_, Visibility, VariantData, Variant, Attribute_, AttrStyle,
                  StrStyle, Lit_, MetaItem_, StructField, Name, Unsafety, ImplPolarity,
                  TraitRef, Ty, Ty_, ImplItem, MethodSig, FnDecl, MutTy, Mutability, FunctionRetTy,
                  ExplicitSelf_, Block, Expr, Expr_, Arm, Pat, Pat_, ImplItemKind, Generics,
                  DUMMY_NODE_ID, BlockCheckMode, Item, Item_, Path, PathSegment,
                  PathParameters, Arg, BindingMode, AngleBracketedParameterData, Delimited, Stmt_,
                  Mac_, FieldPat, StructFieldKind, Field, Constness};
use syntax::abi::Abi;
use syntax::ptr::P;
use syntax::attr::{mk_sugared_doc_attr, mk_attr_id};
use syntax::ext::quote::rt::ToTokens;
use syntax::owned_slice::OwnedSlice;

use rustc_plugin::Registry;

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
  let mut parser = parse::tts_to_parser(cx.parse_sess(), tokens, cx.cfg());

  let mut items: Vec<P<ast::Item>> = Vec::new();
  let mut variants: Vec<VariantDef> = Vec::new();

  let syn_context = type_name.ctxt;

  /*
   * Step 0.
   *
   * Parse the token tree to populate our list of variants.
   *
   */
  loop {
    let var_lo = parser.span.lo;

    // Get the name of this variant.
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
      // It's a unit-like variant. (ie. not a struct variant)
      Ok(Token::FatArrow) => (None, Vec::new()),

      // It's a struct variant
      Ok(Token::OpenDelim(DelimToken::Brace)) => {
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
          let mut from_attr_idx: Option<usize> = None;
          for (i, attr) in attrs.iter().enumerate() {
            if let MetaItem_::MetaWord(ref attr_name) = attr.node.value.node {
              if *attr_name == "from" {   // We've found a #[from] attribute.
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
            // This member is marked #[from]. Record this.
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

          // Parse the name and type of the member.
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

    // Parse the short description.
    let short_desc = match parser.bump_and_get() {
      Ok(Token::Literal(Lit::Str_(sd), None)) => sd,
      _ => {
        let _ = parser.fatal("Expected a string literal");
        return DummyResult::any(sp);
      },
    };

    // Parse the long description if it exists.
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
    }
    else {
      None
    };


    let comment_str = format!("/// {}.", short_desc);
    let comment = InternedString::new_from_name(intern(&comment_str[..]));

    // Build our variant definition out of the information we've parsed.
    variants.push(VariantDef {
      variant: P(spanned(var_lo, var_hi, Variant_ {
        name:      variant_name,
        attrs:     vec![mk_sugared_doc_attr(mk_attr_id(), comment, var_lo, var_hi)],
        data:      match members.len() {
          0 => VariantData::Unit(DUMMY_NODE_ID),
          _ => VariantData::Struct(members, DUMMY_NODE_ID),
        },
        disr_expr: None,
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

  /*
   * Step 1.
   *
   * Now that we have parsed the code, build an AST out of it. 
   *
   */
  let vars = variants.iter().map(|v| v.variant.clone()).collect();

  // Create our enum item.
  items.push(P(Item {
    ident: type_name,
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemEnum(
      EnumDef {
        variants: vars,
      },
      Generics::default()
    ),
    vis:   Visibility::Public,
    span:  DUMMY_SP,
  }));


  // Create an AST for the &str type to use later.
  let str_type = P(Ty {
    id: DUMMY_NODE_ID,
    node: Ty_::TyRptr(None, MutTy {
      ty: P(Ty {
        id: DUMMY_NODE_ID,
        node: Ty_::TyPath(None, path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("str"))])),
        span: DUMMY_SP,
      }),
      mutbl: Mutability::MutImmutable,
    }),
    span: DUMMY_SP,
  });

  // Create an AST for the #[allow(unused_variables)] attr to be used later.
  let unused_attr = dummy_spanned(Attribute_ {
    id: mk_attr_id(),
    style: AttrStyle::Outer,
    value: P(dummy_spanned(MetaItem_::MetaList(
      InternedString::new_from_name(intern("allow")),
      vec![P(dummy_spanned(MetaItem_::MetaWord(InternedString::new_from_name(intern("unused_variables")))))]
    ))),
    is_sugared_doc: false,
  });

  // Create an AST of the method signature of fmt::Display::fmt and fmt::Debug::fmt.
  let fmt_meth_sig = MethodSig {
    unsafety:  Unsafety::Normal,
    constness: Constness::NotConst,
    abi:       Abi::Rust,
    decl:      P(FnDecl {
      inputs:  vec![
        Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_),
        Arg {
          ty: P(Ty {
            id: DUMMY_NODE_ID,
            node: Ty_::TyRptr(None, MutTy {
              ty: P(Ty {
                id:   DUMMY_NODE_ID,
                node: Ty_::TyPath(None, path_from_segments(true, &[
                  ast::Ident::with_empty_ctxt(intern("std")),
                  ast::Ident::with_empty_ctxt(intern("fmt")),
                  ast::Ident::with_empty_ctxt(intern("Formatter")),
                ])),
                span: DUMMY_SP,
              }),
              mutbl: Mutability::MutMutable,
            }),
            span: DUMMY_SP,
          }),
          pat: P(Pat {
            id:   DUMMY_NODE_ID,
            node: Pat_::PatIdent(BindingMode::ByValue(Mutability::MutImmutable), dummy_spanned(ast::Ident::with_empty_ctxt(intern("f"))), None),
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
              identifier: ast::Ident::with_empty_ctxt(intern("std")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::with_empty_ctxt(intern("result")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::with_empty_ctxt(intern("Result")),
              parameters: PathParameters::AngleBracketed(AngleBracketedParameterData {
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
                      ast::Ident::with_empty_ctxt(intern("std")),
                      ast::Ident::with_empty_ctxt(intern("fmt")),
                      ast::Ident::with_empty_ctxt(intern("Error")),
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
    generics: Generics::default(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::with_empty_ctxt(intern("what_is_this")))),
  };

  // Our actual code block for Debug::fmt.
  let debug_fmt_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: Expr_::ExprBlock(P(Block {
      stmts: vec![{
        match v.variant.node.data {
          VariantData::Struct(ref fields, _) => {
            let mut ss = format!("{} {{{{", v.variant.node.name);
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
              node: Expr_::ExprMac(dummy_spanned(Mac_ {
                path: path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("try"))]),
                tts:  vec![
                  TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("write")), IdentStyle::Plain)),
                  TokenTree::Token(DUMMY_SP, Token::Not),
                  TokenTree::Delimited(DUMMY_SP, Rc::new(Delimited {
                    delim: DelimToken::Paren,
                    open_span:  DUMMY_SP,
                    close_span: DUMMY_SP,
                    tts: {
                      let mut tts = vec![
                        TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("f")), IdentStyle::Plain)),
                        TokenTree::Token(DUMMY_SP, Token::Comma),
                        TokenTree::Token(DUMMY_SP, Token::Literal(Lit::Str_(intern(&ss[..])), None)),
                      ];
                      for f in fields.iter() {
                        tts.push(TokenTree::Token(DUMMY_SP, Token::Comma));
                        let field_name = match f.node.kind {
                          StructFieldKind::NamedField(ident, _)  => ident,
                          _                                      => unreachable!(),
                        };
                        tts.push(TokenTree::Token(DUMMY_SP, Token::Ident(field_name, IdentStyle::Plain)));
                      };
                      tts.push(TokenTree::Token(DUMMY_SP, Token::Comma));
                      tts.push(TokenTree::Token(DUMMY_SP, Token::Ident(special_idents::self_, IdentStyle::Plain)));
                      tts
                    },
                  })),
                ],
                ctxt: syn_context
              })),
              attrs: None,
            }), DUMMY_NODE_ID)))
          },
          VariantData::Tuple(..) => unreachable!(),
          VariantData::Unit(_) => {
            let ss = format!("{} /* {{}} */", v.variant.node.name);
            P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
              id:   DUMMY_NODE_ID,
              span: DUMMY_SP,
              node: Expr_::ExprMac(dummy_spanned(Mac_ {
                path: path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("try"))]),
                tts: vec![
                  TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("write")), IdentStyle::Plain)),
                  TokenTree::Token(DUMMY_SP, Token::Not),
                  TokenTree::Delimited(DUMMY_SP, Rc::new(Delimited {
                    delim: DelimToken::Paren,
                    open_span:  DUMMY_SP,
                    close_span: DUMMY_SP,
                    tts: vec![
                      TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("f")), IdentStyle::Plain)),
                      TokenTree::Token(DUMMY_SP, Token::Comma),
                      TokenTree::Token(DUMMY_SP, Token::Literal(Lit::Str_(intern(&ss[..])), None)),
                      TokenTree::Token(DUMMY_SP, Token::Comma),
                      TokenTree::Token(DUMMY_SP, Token::Ident(special_idents::self_, IdentStyle::Plain)),
                    ],
                  })),
                ],
                ctxt: syn_context
              })),
              attrs: None,
            }), DUMMY_NODE_ID)))
          }
        }
      }],
      expr: Some(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCall(
          P(Expr {
            id:    DUMMY_NODE_ID,
            span:  DUMMY_SP,
            node:  Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("Ok"))])),
            attrs: None,
          }),
          vec![
            P(Expr {
              id:    DUMMY_NODE_ID,
              span:  DUMMY_SP,
              node:  Expr_::ExprTup(Vec::new()),
              attrs: None,
            }),
          ]
        ),
        attrs: None,
      })),
      id:    DUMMY_NODE_ID,
      span:  DUMMY_SP,
      rules: BlockCheckMode::DefaultBlock,
    })),
    attrs: None,
  }));

  // The AST for the method implementation of Debug::fmt
  let debug_fmt_impl = P(ImplItem {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    ident: ast::Ident::with_empty_ctxt(intern("fmt")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItemKind::Method(fmt_meth_sig.clone(), debug_fmt_block),
  });

  // The code for Display::fmt
  let display_fmt_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: Expr_::ExprBlock(P(Block {
      stmts: {
        let mut try_writes = vec![
          P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_ {
              path: path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("try"))]),
              tts: vec![
                TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("write")), IdentStyle::Plain)),
                TokenTree::Token(DUMMY_SP, Token::Not),
                TokenTree::Delimited(DUMMY_SP, Rc::new(Delimited {
                  delim: DelimToken::Paren,
                  open_span:  DUMMY_SP,
                  close_span: DUMMY_SP,
                  tts: vec![
                    TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("f")), IdentStyle::Plain)),
                    TokenTree::Token(DUMMY_SP, Token::Comma),
                    TokenTree::Token(DUMMY_SP, Token::Literal(Lit::Str_(v.short_description), None)),
                  ],
                })),
              ],
              ctxt: syn_context
            })),
            attrs: None,
          }), DUMMY_NODE_ID))),
          P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_ {
              path: path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("try"))]),
              tts: vec![
                TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("write")), IdentStyle::Plain)),
                TokenTree::Token(DUMMY_SP, Token::Not),
                TokenTree::Delimited(DUMMY_SP, Rc::new(Delimited {
                  delim: DelimToken::Paren,
                  open_span:  DUMMY_SP,
                  close_span: DUMMY_SP,
                  tts: vec![
                    TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("f")), IdentStyle::Plain)),
                    TokenTree::Token(DUMMY_SP, Token::Comma),
                    TokenTree::Token(DUMMY_SP, Token::Literal(Lit::Str_(intern(". ")), None)),
                  ],
                })),
              ],
              ctxt: syn_context
            })),
            attrs: None,
          }), DUMMY_NODE_ID))),
        ];
        if let Some(ref long_desc) = v.long_description {
          try_writes.push(P(dummy_spanned(Stmt_::StmtSemi(P(Expr {
            id:   DUMMY_NODE_ID,
            span: DUMMY_SP,
            node: Expr_::ExprMac(dummy_spanned(Mac_ {
              path: path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("try"))]),
              tts: vec![
                TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("write")), IdentStyle::Plain)),
                TokenTree::Token(DUMMY_SP, Token::Not),
                TokenTree::Delimited(DUMMY_SP, Rc::new(Delimited {
                  delim: DelimToken::Paren,
                  open_span:  DUMMY_SP,
                  close_span: DUMMY_SP,
                  tts: {
                    let mut write_args = vec![
                      TokenTree::Token(DUMMY_SP, Token::Ident(ast::Ident::with_empty_ctxt(intern("f")), IdentStyle::Plain)),
                      TokenTree::Token(DUMMY_SP, Token::Comma),
                      TokenTree::Token(DUMMY_SP, Token::Literal(Lit::Str_(long_desc.format_str), None)),
                    ];
                    for fa in long_desc.format_args.iter() {
                      write_args.push(TokenTree::Token(DUMMY_SP, Token::Comma));
                      let tt = fa.to_tokens(cx);
                      write_args.extend(tt);
                    };
                    write_args
                  },
                })),
              ],
              ctxt: syn_context
            })),
            attrs: None,
          }), DUMMY_NODE_ID))));
        };
        try_writes
      },
      expr: Some(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCall(
          P(Expr {
            id:    DUMMY_NODE_ID,
            span:  DUMMY_SP,
            node:  Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("Ok"))])),
            attrs: None,
          }),
          vec![
            P(Expr {
              id:    DUMMY_NODE_ID,
              span:  DUMMY_SP,
              node:  Expr_::ExprTup(Vec::new()),
              attrs: None,
            }),
          ]
        ),
        attrs: None,
      })),
      id:    DUMMY_NODE_ID,
      span:  DUMMY_SP,
      rules: BlockCheckMode::DefaultBlock,
    })),
    attrs: None,
  }));

  // The method impl for Display::fmt
  let display_fmt_impl = P(ImplItem {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    ident: ast::Ident::with_empty_ctxt(intern("fmt")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItemKind::Method(fmt_meth_sig, display_fmt_block),
  });

  // AST of the method signature for Error::description
  let description_meth_sig = MethodSig {
    unsafety:  Unsafety::Normal,
    constness: Constness::NotConst,
    abi:       Abi::Rust,
    decl:      P(FnDecl {
      inputs:   vec![Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_)],
      output:   FunctionRetTy::Return(str_type),
      variadic: false,
    }),
    generics: Generics::default(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::with_empty_ctxt(intern("what_is_this")))),
  };

  // The code for our Error::description implementation
  let description_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:    DUMMY_NODE_ID,
    span:  DUMMY_SP,
    node:  Expr_::ExprLit(P(dummy_spanned(Lit_::LitStr(InternedString::new_from_name(v.short_description), StrStyle::CookedStr)))),
    attrs: None,
  }));

  // The method implementation of Error::description
  let description_impl = P(ImplItem {
    id:    DUMMY_NODE_ID,
    span:  DUMMY_SP,
    ident: ast::Ident::with_empty_ctxt(intern("description")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItemKind::Method(description_meth_sig, description_block),
  });

  // AST of the type &Error
  let ref_error_ty = P(Ty {
    node: Ty_::TyRptr(None, MutTy {
      ty: P(Ty {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Ty_::TyPath(None, path_from_segments(true, &[
          ast::Ident::with_empty_ctxt(intern("std")),
          ast::Ident::with_empty_ctxt(intern("error")),
          ast::Ident::with_empty_ctxt(intern("Error")),
        ])),
      }),
      mutbl: Mutability::MutImmutable,
    }),
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
  });

  // AST of the method signature for Error::cause
  let cause_meth_sig = MethodSig {
    unsafety:  Unsafety::Normal,
    constness: Constness::NotConst,
    abi:       Abi::Rust,
    decl:      P(FnDecl {
      inputs:   vec![Arg::new_self(DUMMY_SP, Mutability::MutImmutable, special_idents::self_)],
      output:   FunctionRetTy::Return(P(Ty {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Ty_::TyPath(None, Path {
          span:   DUMMY_SP,
          global: true,
          segments: vec![
            PathSegment {
              identifier: ast::Ident::with_empty_ctxt(intern("std")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::with_empty_ctxt(intern("option")),
              parameters: PathParameters::none(),
            },
            PathSegment {
              identifier: ast::Ident::with_empty_ctxt(intern("Option")),
              parameters: PathParameters::AngleBracketed(AngleBracketedParameterData {
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
    generics: Generics::default(),
    explicit_self: dummy_spanned(ExplicitSelf_::SelfRegion(None, Mutability::MutImmutable, ast::Ident::with_empty_ctxt(intern("what_is_this")))),
  };

  // Code for Error::cause
  let cause_block = mk_match_block(&variants, type_name, |v| P(Expr {
    id:   DUMMY_NODE_ID,
    span: DUMMY_SP,
    node: match v.from_idx {
      Some(i) => Expr_::ExprCall(P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprPath(None, path_from_segments(true, &[
          ast::Ident::with_empty_ctxt(intern("std")),
          ast::Ident::with_empty_ctxt(intern("option")),
          ast::Ident::with_empty_ctxt(intern("Option")),
          ast::Ident::with_empty_ctxt(intern("Some")),
        ])),
        attrs: None,
      }), vec![P(Expr {
        id:   DUMMY_NODE_ID,
        span: DUMMY_SP,
        node: Expr_::ExprCast(P(Expr {
          id:   DUMMY_NODE_ID,
          span: DUMMY_SP,
          node: Expr_::ExprPath(None, path_from_segments(false, &[
            match v.variant.node.data {
              VariantData::Struct(ref fields, _) => match fields[i].node.kind {
                StructFieldKind::NamedField(ident, _) => ident,
                StructFieldKind::UnnamedField(_)      => unreachable!(),
              },
              VariantData::Tuple(..) => unreachable!(),
              VariantData::Unit(_)   => unreachable!(),
            },
          ])),
          attrs: None,
        }), ref_error_ty.clone()),
        attrs: None,
      })]),
      None    => Expr_::ExprPath(None, path_from_segments(true, &[
        ast::Ident::with_empty_ctxt(intern("std")),
        ast::Ident::with_empty_ctxt(intern("option")),
        ast::Ident::with_empty_ctxt(intern("Option")),
        ast::Ident::with_empty_ctxt(intern("None")),
      ])),
    },
    attrs: None,
  }));

  // The method impl for Error::cause
  let cause_impl = P(ImplItem {
    id:    DUMMY_NODE_ID,
    span:  DUMMY_SP,
    ident: ast::Ident::with_empty_ctxt(intern("cause")),
    vis:   Visibility::Inherited,
    attrs: vec![unused_attr.clone()],
    node:  ImplItemKind::Method(cause_meth_sig, cause_block),
  });

  // The AST of our implementation of fmt::Debug
  items.push(P(Item {
    ident: ast::Ident::with_empty_ctxt(intern("whats_this_then")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, Generics::default(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::with_empty_ctxt(intern("std")),
        ast::Ident::with_empty_ctxt(intern("fmt")),
        ast::Ident::with_empty_ctxt(intern("Debug"))
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

  // The AST of our implementation of fmt::Display
  items.push(P(Item {
    ident: ast::Ident::with_empty_ctxt(intern("whats_this_then")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, Generics::default(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::with_empty_ctxt(intern("std")),
        ast::Ident::with_empty_ctxt(intern("fmt")),
        ast::Ident::with_empty_ctxt(intern("Display"))
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

  // The AST of our implementation of error::Error
  items.push(P(Item {
    ident: ast::Ident::with_empty_ctxt(intern("seriously_what_should_this_be")),
    attrs: Vec::new(),
    id:    DUMMY_NODE_ID,
    node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, Generics::default(), Some(TraitRef {
      path:   path_from_segments(true, &[
        ast::Ident::with_empty_ctxt(intern("std")),
        ast::Ident::with_empty_ctxt(intern("error")),
        ast::Ident::with_empty_ctxt(intern("Error"))
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

  // Generate std::convert::From impls for each variant if required.
  for v in variants {
    if let VariantData::Struct(ref fields, _) = v.variant.node.data {
      if fields.len() == 1 && v.from_idx == Some(0) {
        let field = &fields[0].node;
        let from_meth_sig = MethodSig {
          unsafety:      Unsafety::Normal,
          constness:     Constness::NotConst,
          abi:           Abi::Rust,
          decl:          P(FnDecl {
            inputs: vec![Arg {
              ty:  field.ty.clone(),
              pat: P(Pat {
                node: Pat_::PatIdent(BindingMode::ByValue(Mutability::MutImmutable), dummy_spanned(ast::Ident::with_empty_ctxt(intern("e"))), None),
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
          generics:      Generics::default(),
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
                  node:  Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("e"))])),
                  id:    DUMMY_NODE_ID,
                  span:  DUMMY_SP,
                  attrs: None,
                }),
                span: DUMMY_SP,
              }],
              None,
            ),
            attrs: None,
          })),
          id:    DUMMY_NODE_ID,
          rules: BlockCheckMode::DefaultBlock,
          span:  DUMMY_SP,
        });
        let from_meth_impl = P(ImplItem {
          id:    DUMMY_NODE_ID,
          span:  DUMMY_SP,
          ident: ast::Ident::with_empty_ctxt(intern("from")),
          vis:   Visibility::Inherited,
          attrs: Vec::new(),
          node:  ImplItemKind::Method(from_meth_sig, from_meth_block),
        });

        items.push(P(Item {
          ident: ast::Ident::with_empty_ctxt(intern("zoomzoom")),
          attrs: Vec::new(),
          id:    DUMMY_NODE_ID,
          node:  Item_::ItemImpl(Unsafety::Normal, ImplPolarity::Positive, Generics::default(), Some(TraitRef {
            path:   Path {
              span:   DUMMY_SP,
              global: true,
              segments: vec![
                PathSegment {
                  identifier: ast::Ident::with_empty_ctxt(intern("std")),
                  parameters: PathParameters::none(),
                },
                PathSegment {
                  identifier: ast::Ident::with_empty_ctxt(intern("convert")),
                  parameters: PathParameters::none(),
                },
                PathSegment {
                  identifier: ast::Ident::with_empty_ctxt(intern("From")),
                  parameters: PathParameters::AngleBracketed(AngleBracketedParameterData {
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

// helper method for generating a Path from a slice of idents
//
// eg. ["std", "convert", "From"] => std::convert::From
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

// Debug::fmt, Display::fmt, Error::description and Error::cause are all based on a match block
// that descrtructures our error and gets any struct variant members. This implements the common
// code between them. It get's passed a closure that generates an expression from a variant
// definition to use as the result of that variant's match arm.
fn mk_match_block<F>(variants: &Vec<VariantDef>, type_name: ast::Ident, func: F) -> P<Block>
    where F: Fn(&VariantDef) -> P<Expr>
{
  let expr_self = P(Expr {
    id:    DUMMY_NODE_ID,
    node:  Expr_::ExprPath(None, path_from_segments(false, &[ast::Ident::with_empty_ctxt(intern("self"))])),
    span:  DUMMY_SP,
    attrs: None,
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
                    node: match v.variant.node.data {
                      VariantData::Struct(ref fields, _)  => Pat_::PatStruct(
                        path_from_segments(false, &[type_name, v.variant.node.name]),
                        {
                          let mut pat_fields: Vec<Spanned<FieldPat>> = Vec::new();
                          for field in fields.iter() {
                            pat_fields.push(dummy_spanned(FieldPat {
                              ident: match field.node.kind {
                                StructFieldKind::NamedField(ident, _)  => ident,
                                StructFieldKind::UnnamedField(_)       => panic!("not possible"),
                              },
                              pat:  P(Pat {
                                id:   DUMMY_NODE_ID,
                                span: DUMMY_SP,
                                node: Pat_::PatIdent(BindingMode::ByRef(Mutability::MutImmutable), dummy_spanned(match field.node.kind {
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
                      VariantData::Tuple(..) => unreachable!(),
                      VariantData::Unit(_) => Pat_::PatEnum(
                        path_from_segments(false, &[type_name, v.variant.node.name]),
                        Some(Vec::new()),
                      ),
                      /*
                      VariantData::Unit(_) => Pat_::PatEnum(
                        path_from_segments(false, &[type_name, v.variant.node.name]),
                        None,
                      ),
                      */
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
        }
      ),
      span: DUMMY_SP,
      attrs: None,
    })),
    id:    DUMMY_NODE_ID,
    rules: BlockCheckMode::DefaultBlock,
    span:  DUMMY_SP,
  })
}

