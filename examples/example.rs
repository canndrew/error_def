#![feature(plugin)]
#![plugin(error_def)]
#![allow(dead_code)]

use std::io;

error_def! ExampleError {
    AVariant                     => "Unit-like variant",
    AVariantWithALongDescription => "Unit-like variant" ("A more verbose description"),
    AVariantWithArgs {
        flim: u32,
        flam: u32,
    } => "Variant with args" ("This is a format string. flim is {}. flam is {}.", flim, flam),
    AVariantWithACause {
        blah: bool,
        #[from] cause: io::Error,
    } => "Variant with a cause" ("self.cause() would return Some({})", cause),
    AVariantWithJustACause {
        #[from] blah: io::Error,
    } => "This variant can be made `From` an `io::Error`"
}

/* Expands (roughly) to
 
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

*/

#[test]
fn test() {
    use std::error::Error;

    let e = ExampleError::AVariant;
    assert_eq!(&format!("{:?}", e), "AVariant /* Unit-like variant */");
    assert_eq!(&format!("{}", e), "Unit-like variant");
    assert_eq!(e.description(), "Unit-like variant");
    assert!(e.cause().is_none());

    let e = ExampleError::AVariantWithALongDescription;
    assert_eq!(&format!("{:?}", e), "AVariantWithALongDescription /* Unit-like variant. A more verbose description */");
    assert_eq!(&format!("{}", e), "Unit-like variant. A more verbose description");
    assert_eq!(e.description(), "Unit-like variant");
    assert!(e.cause().is_none());

    let e = ExampleError::AVariantWithArgs { flim: 123u32, flam: 456u32 };
    assert_eq!(&format!("{:?}", e), "AVariantWithArgs { flim: 123, flam: 456 } /* Variant with args. This is a format string. flim is 123. flam is 456. */");
    assert_eq!(&format!("{}", e), "Variant with args. This is a format string. flim is 123. flam is 456.");
    assert_eq!(e.description(), "Variant with args");
    assert!(e.cause().is_none());

    let e = ExampleError::AVariantWithACause { blah: true, cause: io::Error::new(io::ErrorKind::PermissionDenied, "Yo you can't do that!") };
    assert_eq!(&format!("{:?}", e), "AVariantWithACause { blah: true, cause: Error { repr: Custom(Custom { kind: PermissionDenied, error: StringError(\"Yo you can\\\'t do that!\") }) } } /* Variant with a cause. self.cause() would return Some(Yo you can't do that!) */");
    assert_eq!(&format!("{}", e), "Variant with a cause. self.cause() would return Some(Yo you can't do that!)");
    assert_eq!(e.description(), "Variant with a cause");
    assert_eq!(e.cause().unwrap().description(), "Yo you can't do that!");

    let e = ExampleError::from(io::Error::new(io::ErrorKind::PermissionDenied, "You still can't do that!"));
    match e {
        ExampleError::AVariantWithJustACause { .. } => (),
        _ => panic!("wtf?"),
    };
    assert_eq!(&format!("{:?}", e), "AVariantWithJustACause { blah: Error { repr: Custom(Custom { kind: PermissionDenied, error: StringError(\"You still can\\\'t do that!\") }) } } /* This variant can be made `From` an `io::Error` */");
    assert_eq!(&format!("{}", e), "This variant can be made `From` an `io::Error`");
    assert_eq!(e.description(), "This variant can be made `From` an `io::Error`");
    assert_eq!(e.cause().unwrap().description(), "You still can't do that!");
}

fn main() {
}

