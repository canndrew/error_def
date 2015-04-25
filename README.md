
## error_def: A Rust syntax extension for generating error-handling boilerplate code.

**Quick Example:** The following code

```rust
error_def! ExampleError {
  AVariant                     => "Unit-like variant",
  AVariantWithALongDescription => "Unit-like variant" ("A more verbose description"),
  AVariantWithArgs {
    flim: u32,
    flam: u32,
  }                     => "Variant with args" ("This is a format string. flim is {}. flam is {}.", flim, flam),
  AVariantWithACause {
    blah: bool,
    #[from] cause: io::Error,
  }                     => "Variant with a cause" ("self.cause() would return Some({})", cause)
  AVariantWithJustACause {
    #[from] blah: io::Error,
  }                     => "This variant can be made `From` an `io::Error`"
}

```

Expands (rougly) to:

```rust
enum ExampleError {
  AVariant,
  AVariantWithALongDescription,
  AVariantWithArgs {
    flim: u32,
    flam: u32,
  },
  AVariantWithACause {
    blah: bool,
    cause: io::Error,
  },
  AVariantWithJustACause {
    blah: io::Error,
  },
}

impl fmt::Debug for ExampleError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result<(), fmt::Error> {
    match self {
      &ExampleError::AVariant                                   => write!(f, "AVariant /* {} */", self),
      &ExampleError::AVariantWithALongDescription               => write!(f, "AVariantWithALongDescription /* {} */", self),
      &ExampleError::AVariantWithArgs { ref flim, ref flam }    => write!(f, "AVariantWithArgs {{ flim: {:?}, flam: {:?} }} /* {} */", flim, flim, self),
      &ExampleError::AVariantWithACause { ref blah, ref cause } => write!(f, "AVariantWithACause {{ blah: {:?}, cause: {:?} }} /* {} */", blah, cause, self),
      &ExampleError::AVariantWithJustACause { ref blah }        => write!(f, "AVariantWithJustACause {{ blah: {:?} }} /* {} */", blah, self),
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
      &ExampleError::AVariantWithJustACause { .. }                       => {
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

**Explanation:** The `error_def` macro defines an enum and creates `impl`s for `std::error::Error`,
`std::fmt::Debug` and `std::fmt::Display`. If a struct variant member is marked with the `#[from]`
pseudo-attribute it will be returned by `Error::cause`. If a struct variant has only one member and
it is marked `#[from]` then `std::convert::From` will be implemented accordingly.

