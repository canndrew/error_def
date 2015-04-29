
## error_def: A Rust syntax extension for generating error-handling boilerplate code.

**Quick Example:** The following code:

```rust
error_def! ExampleError {
  AVariant
    => "Unit-like variant",
  AVariantWithALongDescription
    => "Unit-like variant" ("A more verbose description"),
  AVariantWithArgs { flim: u32, flam: u32 }
    => "Variant with args" ("This is a format string. flim is {}. flam is {}.", flim, flam),
  AVariantWithACause { blah: bool, #[from] cause: io::Error }
    => "Variant with a cause" ("self.cause() would return Some({})", cause)
  AVariantWithJustACause { #[from] blah: io::Error }
    => "This variant can be made `From` an `io::Error`"
}

```

Expands (roughly) to:

```rust
enum ExampleError {
  /// Unit-like variant
  AVariant,

  /// Unit-like variant
  AVariantWithALongDescription,

  /// Variant with args
  AVariantWithArgs {
    flim: u32,
    flam: u32,
  },

  /// Variant with a cause
  AVariantWithACause {
    blah: bool,
    cause: io::Error,
  },

  /// This variant can be made `From` an `io::Error`
  AVariantWithJustACause {
    blah: io::Error,
  },
}

impl fmt::Debug for ExampleError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result<(), fmt::Error> {
    match self {
      &ExampleError::AVariant
        => write!(f, "AVariant /* {} */", self),
      &ExampleError::AVariantWithALongDescription
        => write!(f, "AVariantWithALongDescription /* {} */", self),
      &ExampleError::AVariantWithArgs { ref flim, ref flam }
        => write!(f, "AVariantWithArgs {{ flim: {:?}, flam: {:?} }} /* {} */", flim, flim, self),
      &ExampleError::AVariantWithACause { ref blah, ref cause }
        => write!(f, "AVariantWithACause {{ blah: {:?}, cause: {:?} }} /* {} */", blah, cause, self),
      &ExampleError::AVariantWithJustACause { ref blah }
        => write!(f, "AVariantWithJustACause {{ blah: {:?} }} /* {} */", blah, self),
    }
  }
}

impl fmt::Display for ExampleError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result<(), fmt::Error> {
    match self {
      &ExampleError::AVariant                                => {
        try!(write!(f, "Unit-like variant."));
        Ok(())
      },
      &ExampleError::AVariantWithALongDescription            => {
        try!(write!(f, "Unit-like variant"));
        try!(write!(f, "A more verbose description"));
        Ok(())
      },
      &ExampleError::AVariantWithArgs { ref flim, ref flam } => {
        try!(write!(f, "Variant with args"));
        try!(write!(f, "This is a format string. flim is {}. flam is {}.", flim, flam));
        Ok(())
      },
      &ExampleError::AVariantWithACause { ref cause, .. }    => {
        try!(write!(f, "Variant with a cause"));
        try!(write!(f, "self.cause() would return Some({})", cause));
        Ok(())
      },
      &ExampleError::AVariantWithJustACause { .. }           => {
        try!(write!(f, "This variant can be made `From` an `io::Error`"));
        Ok(())
      },
    }
  }
}

impl Error for ExampleError {
  fn description(&self) -> &str {
    match self {
      &ExampleError::AVariant                            => "Unit-like variant",
      &ExampleError::AVariantWithALongDescription { .. } => "Unit-like variant",
      &ExampleError::AVariantWithArgs { .. }             => "Variant with args",
      &ExampleError::AVariantWithACause { .. }           => "Variant with a cause",
      &ExampleError::AVariantWithJustACause { .. }       => "This variant can be made `From` an `io::Error`",
    }
  }

  fn cause(&self) -> Option<&Error> {
    match self {
      &ExampleError::AVariant                                => None,
      &ExampleError::AVariantWithALongDescription { .. }     => None,
      &ExampleError::AVariantWithArgs { .. }                 => None,
      &ExampleError::AVariantWithACause { ref cause, .. }    => Some(cause as &Error),
      &ExampleError::AVariantWithJustACause { ref blah, .. } => Some(blah as &Error),
    }
  }
}

impl From<io::Error> for ExampleError {
  fn from(e: io::Error) -> ExampleError {
    ExampleError::AVariantWithJustACause { blah: e }
  }
}

```

**Explanation:** `error_def` defines an `enum` where each variant is paired
with a description of the variant

```rust
error_def! SomeError {
  AVariant       => "A description",
  AnotherVariant => "Another description",
}
```

This description is added as a doc-comment to the variant and is returned by
calls to `Error::description`.

```rust
assert!(SomeError::AVariant.description() == "A description")
```

Variants can be struct-like.

```rust
error_def! SomeError {
  AVariant { an_i32: i32 }  => "I'm a variant",
}
```

Variants can also have an optional long-description which consists of a format
string and a sequence of arguments. The long description is placed in
parenthesis after the short-description. If the variant is a struct, the
arguments to the format string can refer to it's members.

```rust
error_def! SomeError {
  Io { cause: io::Error }
    => "I/O error occured!" ("Error: {}", cause),
}
```

`error_def!` uses the short and long descriptions to provide `impl`s of
`fmt::Debug` and `fmt::Display`. In the above case, `SomeError::Io` would be
formatted as

`"SomeError::Io { cause: `*`io:Error Debug formatted here`*` } /* I/O error occured. Error: `*`io::Error displayed here`*` */"`

For `fmt::Debug` and

`"I/O error occured. Error: `*`io::Error displayed here`*`"`

For `fmt::Display`.

Members of a struct-variant can be marked with an optional `#[from]` pseudo-attribute.

```rust
error_def! SomeError {
  Io {
    foo: u32,
    #[from] cause: io::Error,
  } => "Io error"
}
```

This causes the member to be returned by calls to `Error::cause`. In the above
example, calling `Error::cause` on a `SomeError::Io` will return an
`Option<&Error>` where the `&Error` points to an `io::Error`.

If a struct variant has only one member and it is marked `#[from]` then `From`
will be implemented to cast the type of that member to the type of the error.

For example, if we define an error like this:

```rust 
error_def! SomeError {
  Io { #[from] cause: io::Error } => "I/O error",
}
```

Then `error_def!` will define an `impl`:

```rust
impl std::convert::From<io::Error> for SomeError {
  fn from(e: io::Error) -> SomeError {
    SomeError::Io {
      cause: e,
    }
  }
}
```

