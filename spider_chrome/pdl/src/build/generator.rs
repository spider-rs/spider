use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::ops::Deref;
use std::path::{Path, PathBuf};

use either::Either;
use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::build::builder::Builder;
use crate::build::event::{EventBuilder, EventType};
use crate::build::types::*;
use crate::pdl::parser::parse_pdl;
use crate::pdl::{DataType, Domain, Param, Protocol, Type, Variant};

/// Compile `.pdl` files into Rust files during a Cargo build.
///
/// The generated `.rs` files are written to the Cargo `OUT_DIR` directory,
/// suitable for use with
///
/// This function should be called in a project's `build.rs`.
///
/// # Arguments
///
/// **`pdls`** - Paths to `.pdl` files to compile.
///
/// # Errors
///
/// This function can fail for a number of reasons:
///
///   - Failure to locate `pdl` files.
///   - Failure to parse the `.pdl`s.
///
/// It's expected that this function call be `unwrap`ed in a `build.rs`; there
/// is typically no reason to gracefully recover from errors during a build.
///
/// # Example `build.rs`
///
/// ```rust,no_run
/// # use std::io::Result;
/// fn main() -> Result<()> {
///   chromiumoxide_pdl::build::compile_pdls(&["src/js.pdl", "src/browser.pdl"])?;
///   Ok(())
/// }
/// ```
pub fn compile_pdls<P: AsRef<Path>>(pdls: &[P]) -> io::Result<()> {
    Generator::default().compile_pdls(pdls)
}

/// Generates rust code for the Chrome DevTools Protocol
#[derive(Debug, Clone)]
pub struct Generator {
    serde_support: SerdeSupport,
    with_experimental: bool,
    with_deprecated: bool,
    out_dir: Option<PathBuf>,
    protocol_mods: Vec<String>,
    domains: HashMap<String, usize>,
    target_mod: Option<String>,
    /// Used to store the size of a specific type
    type_size: HashMap<String, usize>,
    /// Used to fix a type's size later if the ref was not processed yet
    ref_sizes: VecDeque<(String, String)>,
    /// This contains a list of all enums of all domains with their qualified
    /// names <domain>.<name>
    ///
    /// This is a fix in order to check in struct definitions whether the
    /// targeted type is an enum
    enums: HashSet<String>,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            serde_support: Default::default(),
            with_experimental: true,
            with_deprecated: false,
            out_dir: None,
            protocol_mods: Vec::new(),
            domains: Default::default(),
            target_mod: Default::default(),
            type_size: Default::default(),
            ref_sizes: VecDeque::new(),
            enums: Default::default(),
        }
    }
}

impl Generator {
    /// Configures the output directory where generated Rust files will be
    /// written.
    ///
    /// If unset, defaults to the `OUT_DIR` environment variable. `OUT_DIR` is
    /// set by Cargo when executing build scripts, so `out_dir` typically
    /// does not need to be configured.
    pub fn out_dir<P>(&mut self, path: P) -> &mut Self
    where
        P: Into<PathBuf>,
    {
        self.out_dir = Some(path.into());
        self
    }

    /// Configures the serde support that should be included for all the
    /// generated types.
    pub fn serde(&mut self, serde: SerdeSupport) -> &mut Self {
        self.serde_support = serde;
        self
    }

    /// Configures whether experimental types and fields should be included.
    ///
    /// Disabling experimental types may result in missing type definitions
    /// (E0412)
    pub fn experimental(&mut self, experimental: bool) -> &mut Self {
        self.with_experimental = experimental;
        self
    }

    /// Configures whether deprecated types and fields should be included.
    pub fn deprecated(&mut self, deprecated: bool) -> &mut Self {
        self.with_deprecated = deprecated;
        self
    }

    /// Configures the name of the module and file generated.
    pub fn target_mod(&mut self, mod_name: impl Into<String>) -> &mut Self {
        self.target_mod = Some(mod_name.into());
        self
    }

    /// Compile `.pdls` files into Rust files during a Cargo build with
    /// additional code generator configuration options.
    ///
    /// This method is like the `chromiumoxide_pdl::build::compile_pdls`
    /// function, with the added ability to specify non-default code
    /// generation options. See that function for more information about the
    /// arguments and generated outputs.
    ///
    /// # Example `build.rs`
    ///
    /// ```rust,no_run
    /// # use std::io::Result;
    /// fn main() -> Result<()> {
    ///   let mut pdl_build = chromiumoxide_pdl::build::Generator::default();
    ///   pdl_build.out_dir("some/path");
    ///   pdl_build.compile_pdls(&["src/frontend.pdl", "src/backend.pdl"])?;
    ///   Ok(())
    /// }
    /// ```
    pub fn compile_pdls<P: AsRef<Path>>(&mut self, pdls: &[P]) -> io::Result<()> {
        let target: PathBuf = self.out_dir.clone().map(Ok).unwrap_or_else(|| {
            std::env::var_os("OUT_DIR")
                .ok_or_else(|| {
                    Error::new(ErrorKind::Other, "OUT_DIR environment variable is not set")
                })
                .map(Into::into)
        })?;

        let mut inputs = vec![];

        for path in pdls {
            let path = path.as_ref();
            let file_name = path.file_stem().ok_or_else(|| {
                Error::new(
                    ErrorKind::Other,
                    format!("Failed to read file name for {}", path.display()),
                )
            })?;
            let mod_name = file_name.to_string_lossy().to_string();
            self.protocol_mods.push(mod_name);

            inputs.push(fs::read_to_string(path)?);
        }

        let mut protocols = vec![];

        for (idx, input) in inputs.iter().enumerate() {
            let pdl = parse_pdl(input).map_err(|e| Error::new(ErrorKind::Other, e.message))?;

            self.domains
                .extend(pdl.domains.iter().map(|d| (d.name.to_string(), idx)));

            // store enum types
            self.enums.extend(
                pdl.domains
                    .iter()
                    .flat_map(|d| d.types.iter().filter(|d| d.is_enum()))
                    .map(|e| e.raw_name.to_string()),
            );

            protocols.push(pdl);
        }

        let mut modules = TokenStream::default();

        for (idx, pdl) in protocols.iter().enumerate() {
            let types = self.generate_types(&pdl.domains);
            let version = format!("{}.{}", pdl.version.major, pdl.version.minor);
            let module_name = format_ident!("{}", self.protocol_mods[idx]);
            let module = quote! {
                #[allow(clippy::wrong_self_convention)]
                pub mod #module_name{
                    /// The version of this protocol definition
                    pub const VERSION : &str = #version;
                    #types
                }
            };

            modules.extend(module);
        }

        // brute-force fix unresolved type sizes
        let mut refs = std::mem::take(&mut self.ref_sizes);
        let mut sequential_retries = 0;
        while let Some((name, reff)) = refs.pop_front() {
            if let Some(ref_size) = self.type_size.get(&reff).copied() {
                sequential_retries = 0;
                self.store_size(&name, Either::Left(ref_size));
            } else {
                sequential_retries += 1;
                if sequential_retries > refs.len() {
                    panic!("No type found for ref {reff}");
                }
                refs.push_back((name, reff));
            }
        }

        let mod_name = self.target_mod.as_deref().unwrap_or("cdp");
        let events = self.generate_event_enums(&protocols);
        let imports = self.serde_support.generate_serde_import_deserialize();
        let stream = quote! {
            /// This file is generated and should not be edited directly.
            pub use events::*;

            /// This trait allows for implementing custom events that are not covered by the
            /// chrome protocol definitions.
            ///
            /// Every `CustomEvent` also requires an implementation of
            /// `chromiumoxide_types::MethodType` and it must be `DeserializeOwned`
            /// (`#[derive(serde::Deserialize)]`). This is necessary to identify match this
            /// type against the provided `method` identifier of a `CdpEventMessage`
            /// and to properly deserialize it from a `serde_json::Value`
            pub trait CustomEvent: ::std::any::Any + serde::de::DeserializeOwned + chromiumoxide_types::MethodType + Send + Sync {

                /// Used to convert the json event into in instance of this type
                fn from_json(event: serde_json::Value) -> serde_json::Result<Self> where Self: Sized + 'static {
                        serde_json::from_value(event)
                }
            }

            impl<T:CustomEvent> sealed::SealedEvent for T {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self
                }
            }

            /// This is trait that all Events share
            ///
            /// This trait is sealed to prevent implementation. The only way to implement a new `Event` is by implementing `CustomEvent`
            pub trait Event: sealed::SealedEvent {}

            impl<T: sealed::SealedEvent> Event for T {}
            impl<T: CustomEvent + Event> sealed::SealedCustomEventConverter for T {}

            /// Function type to convert a json event into an instance of it self but as dyn Event
                pub type EventConversion = Box<dyn Fn(serde_json::Value) -> serde_json::Result<::std::sync::Arc<dyn Event>> + Send + 'static>;

            /// An enum that does nothing for built in types but contains the conversion method for custom events
            pub enum EventKind {
                BuiltIn,
                Custom(EventConversion)
            }

            impl EventKind {

                /// Whether this is a custom event
                pub fn is_custom(&self) -> bool {
                        matches!(self,EventKind::Custom(_))
                }

            }

            impl ::std::fmt::Debug for EventKind {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    match self {
                        EventKind::BuiltIn => {
                                f.debug_tuple("BuiltIn").finish()
                        }
                        EventKind::Custom(_) => {
                            f.debug_tuple("Custom").finish()
                        }
                    }
                }
            }

            /// A trait on top of the `Event` trait
            pub trait IntoEventKind : Event {
                /// What kind of event this type is
                fn event_kind() -> EventKind where Self : Sized + 'static;
            }

            impl<T: sealed::SealedCustomEventConverter > IntoEventKind for T {
                fn event_kind() -> EventKind where Self : Sized + 'static{
                    EventKind::Custom(Box::new(Self::event_arc))
                }
            }

            pub(crate) mod sealed {

                pub trait SealedCustomEventConverter : super::CustomEvent + super::Event {

                        fn event_arc(event: serde_json::Value) -> serde_json::Result<::std::sync::Arc<dyn super::Event>> where Self: Sized + 'static {
                            Ok(::std::sync::Arc::new(Self::from_json(event)?))
                        }
                }

                pub trait SealedEvent: ArcAny + chromiumoxide_types::MethodType {
                    /// generate `&::std::any::Any`'s vtable from `&Trait`'s.
                    fn as_any(&self) -> &dyn ::std::any::Any;
                }

                pub trait ArcAny: ::std::any::Any + Send + Sync {
                    fn into_any_arc(self: ::std::sync::Arc<Self>) -> ::std::sync::Arc<dyn ::std::any::Any + Send + Sync>;
                }

                impl<T: ::std::any::Any + Send + Sync> ArcAny for T {
                    fn into_any_arc(self: ::std::sync::Arc<Self>) -> ::std::sync::Arc<dyn ::std::any::Any + Send + Sync> {
                        self
                    }
                }

                impl dyn SealedEvent {
                    /// Returns true if the trait object wraps an object of type `T`.
                    #[inline]
                    pub fn is<T: SealedEvent>(&self) -> bool {
                        self.as_any().is::<T>()
                    }

                    #[inline]
                    pub fn downcast_arc<T: SealedEvent>(self: ::std::sync::Arc<Self>) -> Result<::std::sync::Arc<T>, ::std::sync::Arc<Self>>
                        where
                            T: ::std::any::Any + Send + Sync,
                    {
                        if self.is::<T>() {
                            Ok(ArcAny::into_any_arc(self).downcast::<T>().unwrap())
                        } else {
                            Err(self)
                        }
                    }
                }
            }

            pub mod events {
                #imports
                #events
            }
            #modules

            pub mod de {
                use serde::{de, Deserialize, Deserializer};
                use std::str::FromStr;

                /// Use the `FromStr` implementation to serialize an optional value
                pub fn deserialize_from_str_optional<'de, D, T>(data: D) -> Result<Option<T>, D::Error>
                    where
                        D: Deserializer<'de>,
                        T: FromStr<Err = String>,
                {
                    deserialize_from_str(data).map(Some)
                }

                /// Use the `FromStr` implementation to serialize a value
                pub fn deserialize_from_str<'de, D, T>(data: D) -> Result<T, D::Error>
                    where
                        D: Deserializer<'de>,
                        T: FromStr<Err = String>,
                {
                    let s: String = Deserialize::deserialize(data)?;
                    T::from_str(&s).map_err(de::Error::custom)
                }
            }
        };

        let output = target.join(format!("{mod_name}.rs"));
        fs::write(output, stream.to_string())?;

        fmt(target);
        Ok(())
    }

    /// Generate the types for the domains.
    ///
    /// Each domain gets it's own module
    fn generate_types(&mut self, domains: &[Domain]) -> TokenStream {
        let mut modules = TokenStream::default();
        let with_deprecated = self.with_deprecated;
        let with_experimental = self.with_experimental;
        for domain in domains
            .iter()
            .filter(|d| with_deprecated || !d.deprecated)
            .filter(|d| with_experimental || !d.experimental)
        {
            let domain_mod = self.generate_domain(domain);
            let mod_name = format_ident!("{}", domain.name.to_snake_case());

            let mut desc = if let Some(desc) = domain.description.as_ref() {
                quote! {
                    #[doc = #desc]
                }
            } else {
                TokenStream::default()
            };

            if domain.deprecated {
                desc.extend(quote! {#[deprecated]})
            }

            modules.extend(quote! {
                #desc
                pub mod #mod_name {
                    #domain_mod
                }
            });
        }
        modules
    }

    /// Generates all types are not circular for a single domain
    pub fn generate_domain(&mut self, domain: &Domain) -> TokenStream {
        let mut stream = self.serde_support.generate_serde_imports();
        let with_deprecated = self.with_deprecated;
        let with_experimental = self.with_experimental;
        stream.extend(
            domain
                .into_iter()
                .filter(|dt| with_deprecated || !dt.is_deprecated())
                .filter(|dt| with_experimental || !dt.is_experimental())
                .map(|ty| self.generate_type(domain, ty)),
        );
        stream
    }

    /// Generates all rust types for a PDL `DomainDatatype` (Command, Event,
    /// Type)
    fn generate_type(&mut self, domain: &Domain, dt: DomainDatatype) -> TokenStream {
        let stream = if let Some(vars) = dt.as_enum() {
            self.generate_enum(&Variant::from(&dt), vars)
        } else {
            let with_deprecated = self.with_deprecated;
            let with_experimental = self.with_experimental;
            let params = dt
                .params()
                .filter(|dt| with_deprecated || !dt.is_deprecated())
                .filter(|dt| with_experimental || !dt.is_experimental());

            let mut stream = self.generate_struct(domain, &dt, dt.ident_name(), params);
            let identifier = dt.raw_name();
            let name = format_ident!("{}", dt.ident_name());
            stream.extend(quote! {
              impl #name {
                  pub const IDENTIFIER : &'static str = #identifier;
              }
            });
            if !dt.is_type() {
                stream.extend(quote! {
                    impl chromiumoxide_types::Method for #name {

                        fn identifier(&self) -> chromiumoxide_types::MethodId {
                            Self::IDENTIFIER.into()
                        }
                    }

                    impl chromiumoxide_types::MethodType for #name {

                        fn method_id() -> chromiumoxide_types::MethodId where Self: Sized {
                            Self::IDENTIFIER.into()
                        }
                    }
                });
            }

            if let DomainDatatype::Commnad(cmd) = dt {
                let returns_name = format!("{}Returns", cmd.name().to_upper_camel_case());
                let with_deprecated = self.with_deprecated;
                let with_experimental = self.with_experimental;

                stream.extend(
                    self.generate_struct(
                        domain,
                        &dt,
                        returns_name,
                        cmd.returns
                            .iter()
                            .filter(|p| with_deprecated || !p.is_deprecated())
                            .filter(|p| with_experimental || !p.is_experimental()),
                    ),
                );

                // impl `Command` trait
                let response = format_ident!("{}Returns", dt.name().to_upper_camel_case());
                stream.extend(quote! {
                    impl chromiumoxide_types::Command for #name {
                        type Response = #response;
                    }
                });
            }
            stream
        };
        if dt.is_deprecated() {
            quote! {
                #[deprecated]
                #stream
            }
        } else {
            stream
        }
    }

    fn store_size(&mut self, ty: &str, size: Either<usize, String>) {
        match size {
            Either::Left(size) => {
                let s = self.type_size.entry(ty.to_string()).or_default();
                *s += size;
            }
            Either::Right(name) => self.ref_sizes.push_back((ty.to_string(), name)),
        }
    }

    /// Entry point to modify the builder for a struct manually
    ///
    /// This is useful to add utility fields that should not be serialized by
    /// make things easier
    fn apply_struct_fixup(&self, builder: &mut Builder, dt: &DomainDatatype) {
        if dt.raw_name() == "Runtime.evaluate" {
            let field = FieldDefinition {
                name: "eval_as_function_fallback".to_string(),
                name_ident: format_ident!("eval_as_function_fallback"),
                ty: FieldType {
                    needs_box: false,
                    is_vec: false,
                    ty: quote! {
                        bool
                    },
                },
                optional: true,
                deprecated: false,
                is_enum: false,
                serde_skip: true,
            };

            let def = field.field_definition();

            let meta = quote! {
                /// This is a manually added field that is not part of the protocol definition, hence ignored during serde operations.
                ///
                /// If set to true, this field indicates, that if the command resulted in a response value of type `function` this, `EvaluateParams` command should be executed as a `CallFunctionOnParams` instead.
                #[serde(skip)]
                #def
            };
            builder.fields.push((meta, field));
        }
    }

    /// Generates the struct definitions including enum definitions inner
    /// parameter enums
    fn generate_struct<'a, T>(
        &mut self,
        domain: &Domain,
        dt: &DomainDatatype,
        struct_ident: String,
        params: T,
    ) -> TokenStream
    where
        T: Iterator<Item = &'a Param<'a>> + 'a,
    {
        let name = format_ident!("{}", struct_ident);
        // also generate enums for inner enums
        let mut enum_definitions = TokenStream::default();
        let mut builder = Builder::new(name.clone());

        for param in params {
            if let Type::Enum(vars) = &param.r#type {
                let enum_ident = Variant {
                    description: param.description().map(Cow::Borrowed),
                    name: Cow::Owned(subenum_name(dt.name(), param.name())),
                };
                if param.is_deprecated() {
                    enum_definitions.extend(quote! {#[deprecated]});
                }
                enum_definitions.extend(self.generate_enum(&enum_ident, vars));
            }

            let field_name = format_ident!("{}", generate_field_name(param.name()));

            let (ty, size) =
                self.generate_field_type(domain, dt.name(), param.name(), &param.r#type);
            self.store_size(&struct_ident, size);

            // check if the type of the param points to an enum
            let is_enum = if let Type::Ref(name) = &param.r#type {
                self.enums.contains(name.as_ref())
                    || self
                        .enums
                        .contains(&format!("{}.{}", domain.name, name.as_ref()))
            } else {
                param.r#type.is_enum()
            };

            let field = FieldDefinition {
                name: param.name().to_string(),
                name_ident: field_name,
                ty,
                optional: param.optional,
                deprecated: param.is_deprecated(),
                is_enum,
                serde_skip: false,
            };

            builder
                .fields
                .push((field.generate_meta(&self.serde_support, param), field));
        }

        self.apply_struct_fixup(&mut builder, dt);

        let derives = if !builder.has_mandatory_types() {
            quote! { #[derive(Debug, Clone, PartialEq, Default)]}
        } else {
            quote! {#[derive(Debug, Clone, PartialEq)] }
        };

        let serde_derives = self.serde_support.generate_derives();

        let desc = dt.type_description_tokens(domain.name.as_ref());

        let mut stream = quote! {
            #desc
            #derives
            #serde_derives
        };

        if builder.fields.is_empty() {
            if let DomainDatatype::Type(tydef) = dt {
                // create wrapper types if no fields present
                let (wrapped_ty, size) =
                    self.generate_field_type(domain, dt.name(), dt.name(), &tydef.extends);
                self.store_size(&struct_ident, size);
                let struct_def = quote! {
                    pub struct #name( #wrapped_ty);

                    impl #name {

                        pub fn new(val: impl Into<#wrapped_ty>) -> Self {
                            #name(val.into())
                        }

                        pub fn inner(&self) -> &#wrapped_ty {
                            &self.0
                        }
                    }
                };

                // add Hash +  Eq for integer and string types
                if tydef.extends.is_integer() {
                    stream.extend(quote! {
                        #[derive(Eq, Copy, Hash)]
                        #struct_def
                    });
                } else if tydef.extends.is_string() {
                    // add string helpers
                    stream.extend(quote! {
                        #[derive(Eq, Hash)]
                        #struct_def

                        impl AsRef<str> for #name {
                            fn as_ref(&self) -> &str {
                                self.0.as_str()
                            }
                        }

                        impl From<#name> for String {
                            fn from(el: #name) -> String {
                                el.0
                            }
                        }

                        impl From<String> for #name {
                            fn from(expr: String) -> Self {
                                #name(expr)
                            }
                        }
                    });
                    // Fixup specifically types used as keys
                    if struct_ident.ends_with("Id") {
                        stream.extend(quote! {
                            impl std::borrow::Borrow<str> for #name {
                                fn borrow(&self) -> &str {
                                    &self.0
                                }
                            }
                        })
                    }
                } else {
                    stream.extend(struct_def);
                }
            } else {
                // zero sized struct
                self.type_size.insert(struct_ident, 0);
                stream.extend(quote! {
                    pub struct #name {}
                })
            }
        } else {
            let struct_def = builder.generate_struct_def();
            stream.extend(quote! {
                #struct_def
                #enum_definitions
            });

            if dt.is_command() || dt.is_type() {
                stream.extend(builder.generate_impl());
            }
        }
        stream
    }

    /// Generate enum type with `as_str` and `FromStr` methods
    fn generate_enum(&mut self, ident: &Variant, variants: &[Variant]) -> TokenStream {
        let enum_name = ident
            .name
            .as_ref()
            .rsplit('.')
            .next()
            .unwrap()
            .to_upper_camel_case();

        let name = format_ident!("{}", enum_name);

        self.type_size.insert(enum_name, 16);

        let vars = variants
            .iter()
            .map(|v| self.serde_support.generate_variant(v));

        let desc = if let Some(desc) = ident.description.as_ref() {
            quote! {
                #[doc = #desc]
            }
        } else {
            TokenStream::default()
        };

        let attr = self.serde_support.generate_derives();

        let ty_def = quote! {
            #desc
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            #attr
            pub enum #name {
                #(#vars),*
            }
        };

        // from str to string impl
        let vars: Vec<_> = variants
            .iter()
            .map(|s| format_ident!("{}", generate_enum_field_name(&s.name)))
            .collect();

        let str_values: Vec<_> = variants
            .iter()
            .map(|s| {
                let mut vars = vec![s.name.to_string()];
                let lc = s.name.to_lowercase();
                let cc = generate_enum_field_name(&s.name);
                if cc != lc && vars[0] != cc {
                    vars.push(cc);
                }
                if vars[0] != lc {
                    vars.push(lc);
                }
                vars
            })
            .collect();

        let str_fns = generate_enum_str_fns(&name, &vars, &str_values);

        quote! {
            #ty_def
            #str_fns
        }
    }

    /// Generates the Tokenstream for the field type (bool, f64, etc.)
    fn generate_field_type(
        &self,
        domain: &Domain,
        parent: &str,
        param_name: &str,
        ty: &Type,
    ) -> (FieldType, Either<usize, String>) {
        use std::mem::size_of;
        match ty {
            Type::Integer => (
                FieldType::new(quote! {
                    i64
                }),
                Either::Left(size_of::<i64>()),
            ),
            Type::Number => (
                FieldType::new(quote! {
                    f64
                }),
                Either::Left(size_of::<f64>()),
            ),
            Type::Boolean => (
                FieldType::new(quote! {
                    bool
                }),
                Either::Left(size_of::<bool>()),
            ),
            Type::String => (
                FieldType::new(quote! {
                    String
                }),
                Either::Left(size_of::<String>()),
            ),
            Type::Object | Type::Any => (
                FieldType::new(quote! {serde_json::Value}),
                Either::Left(size_of::<serde_json::Value>()),
            ),
            Type::Binary => (
                FieldType::new(quote! {chromiumoxide_types::Binary}),
                Either::Left(size_of::<chromiumoxide_types::Binary>()),
            ),
            Type::Enum(_) => {
                let ty = format_ident!("{}", subenum_name(parent, param_name));
                (FieldType::new(quote! {#ty}), Either::Left(16))
            }
            Type::ArrayOf(ty) => {
                // recursive types don't need to be boxed in vec
                let ty = if let Type::Ref(name) = ty.deref() {
                    self.projected_type(domain, name)
                } else {
                    let (ty, _) = self.generate_field_type(domain, parent, param_name, ty);
                    quote! {#ty}
                };
                (FieldType::new_vec(ty), Either::Left(size_of::<Vec<()>>()))
            }
            Type::Ref(name) => {
                // consider recursive types
                if name == parent {
                    let ident = format_ident!("{}", name.to_upper_camel_case());
                    (
                        FieldType::new_box(quote! {
                           #ident
                        }),
                        Either::Left(size_of::<Box<()>>()),
                    )
                } else {
                    (
                        FieldType::new(self.projected_type(domain, name)),
                        Either::Right(
                            name.rsplit('.')
                                .next()
                                .unwrap()
                                .to_string()
                                .to_upper_camel_case(),
                        ),
                    )
                }
            }
        }
    }

    /// Resolve projections: `Runtime.ScriptId` where `Runtime` is the
    /// referenced domain where `ScriptId` is defined.
    ///
    /// In order to resolve cross pdl references a domain check is necessary.
    /// If the referenced domain is defined in another pdl than the `domain`'s
    /// pdl, we need to move up an additional level (`super::super`)
    fn projected_type(&self, domain: &Domain, name: &str) -> TokenStream {
        let mut iter = name.rsplitn(2, '.');
        let ty_name = iter.next().unwrap();
        let path = iter.collect::<String>();
        let ident = format_ident!("{}", ty_name.to_upper_camel_case());
        if path.is_empty() {
            quote! {
                #ident
            }
        } else {
            let current_domain_idx = self.domains.get(domain.name.as_ref()).unwrap();
            let ref_domain_idx = self
                .domains
                .get(&path)
                .unwrap_or_else(|| panic!("No referenced domain found for {path}"));

            if *current_domain_idx == *ref_domain_idx {
                let super_ident = format_ident!("{}", path.to_snake_case());
                quote! {
                    super::#super_ident::#ident
                }
            } else {
                let mod_name = format_ident!("{}", self.protocol_mods[*ref_domain_idx]);
                let super_ident = format_ident!("{}", path.to_snake_case());
                quote! {
                    super::super::#mod_name::#super_ident::#ident
                }
            }
        }
    }

    fn generate_event_enums(&self, pdls: &[Protocol]) -> TokenStream {
        let mut events = Vec::new();
        for domain in pdls.iter().flat_map(|p| {
            p.domains
                .iter()
                .filter(|d| self.with_deprecated || !d.deprecated)
                .filter(|d| self.with_experimental || !d.experimental)
        }) {
            for event in domain
                .into_iter()
                .filter_map(|d| {
                    if let DomainDatatype::Event(ev) = d {
                        Some(ev)
                    } else {
                        None
                    }
                })
                .filter(|ev| self.with_deprecated || !ev.is_deprecated())
                .filter(|ev| self.with_experimental || !ev.is_experimental())
            {
                let domain_idx = self
                    .domains
                    .get(domain.name.as_ref())
                    .unwrap_or_else(|| panic!("No matching domain registered for {}", domain.name));
                let protocol_mod = format_ident!("{}", self.protocol_mods[*domain_idx]);

                let ev_name = format!("Event{}", event.name().to_upper_camel_case());

                let size = *self
                    .type_size
                    .get(&ev_name)
                    .unwrap_or_else(|| panic!("No type found for ref {ev_name}"));

                // See https://rust-lang.github.io/rust-clippy/master/#large_enum_variant
                // The maximum size of a enum’s variant to avoid box suggestion is 200
                let needs_box = size > 200;

                events.push(EventType {
                    protocol_mod,
                    domain,
                    inner: event,
                    needs_box,
                });
            }
        }
        EventBuilder::new(events).build()
    }
}

fn generate_enum_str_fns(name: &Ident, vars: &[Ident], str_vals: &[Vec<String>]) -> TokenStream {
    assert_eq!(vars.len(), str_vals.len());
    let mut from_str_stream = TokenStream::default();
    let mut as_str_idents = Vec::new();
    for (var, strs) in vars.iter().zip(str_vals.iter()) {
        from_str_stream.extend(quote! {
                #(#strs)|* => Ok(#name::#var),
        });
        as_str_idents.push(&strs[0]);
    }

    quote! {
        impl AsRef<str> for #name {
            fn as_ref(&self) -> &str {
                match self {
                    #( #name::#vars => #as_str_idents ),*
                }
            }
        }

        impl ::std::str::FromStr for #name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    #from_str_stream
                    _=> Err(s.to_string())
                }
            }
        }
    }
}

/// Escapes reserved rust keywords
pub(crate) fn generate_field_name(name: &str) -> String {
    let name = name.to_snake_case();
    match name.as_str() {
        "type" => "r#type".to_string(),
        "mod" => "r#mod".to_string(),
        "override" => "r#override".to_string(),
        _ => name,
    }
}

pub(crate) fn generate_enum_field_name(name: &str) -> String {
    match name {
        "Self" => "KSelf".to_string(),
        _ => name.to_upper_camel_case(),
    }
}

/// Creates the name for an enum defined inside a type
///
/// ```text
/// type Parent
///     enum type
/// ```
/// to `ParentType`
fn subenum_name(parent: &str, inner: &str) -> String {
    format!(
        "{}{}",
        parent.to_upper_camel_case(),
        generate_enum_field_name(inner)
    )
}

#[derive(Debug, Default, Clone)]
pub enum SerdeSupport {
    None,
    #[default]
    Default,
    Feature(String),
}

impl SerdeSupport {
    pub fn with_feature(feature: impl Into<String>) -> Self {
        SerdeSupport::Feature(feature.into())
    }

    fn generate_serde_import_deserialize(&self) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                 use serde::Deserialize;
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                    #[cfg(feature = #feature)]
                    use serde::Deserialize;
                }
            }
        }
    }

    fn generate_serde_imports(&self) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                 use serde::{Serialize, Deserialize};
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                    #[cfg(feature = #feature)]
                    use serde::{Serialize, Deserialize};
                }
            }
        }
    }

    fn generate_derives(&self) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                #[derive(Serialize, Deserialize)]
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                    #[cfg_attr(feature = #feature, derive(Serialize, Deserialize))]
                }
            }
        }
    }

    pub(crate) fn generate_opt_field_attr(&self) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                 #[serde(skip_serializing_if = "Option::is_none")]
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                     #[cfg_attr(feature = #feature, serde(skip_serializing_if = "Option::is_none"))]
                }
            }
        }
    }

    pub(crate) fn generate_vec_field_attr(&self) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                 #[serde(skip_serializing_if = "Vec::is_empty")]
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                     #[cfg_attr(feature = #feature, serde(skip_serializing_if = "Vec::is_empty"))]
                }
            }
        }
    }

    pub(crate) fn generate_enum_de_with(is_option: bool) -> TokenStream {
        if is_option {
            // NOTE: `#[serde(default)]` is needed here: https://stackoverflow.com/a/44303505/6242846
            quote! {
                 #[serde(default)]
                 #[serde(deserialize_with = "super::super::de::deserialize_from_str_optional")]
            }
        } else {
            quote! {
                 #[serde(deserialize_with = "super::super::de::deserialize_from_str")]
            }
        }
    }

    pub(crate) fn generate_rename(&self, rename: &str) -> TokenStream {
        match self {
            SerdeSupport::None => TokenStream::default(),
            SerdeSupport::Default => quote! {
                 #[serde(rename = #rename)]
            },
            SerdeSupport::Feature(feature) => {
                quote! {
                     #[cfg_attr(feature = #feature, serde(rename = #rename))]
                }
            }
        }
    }

    fn generate_variant(&self, var: &Variant) -> TokenStream {
        let v = format_ident!("{}", generate_enum_field_name(&var.name));
        let rename = self.generate_rename(var.name.as_ref());
        if let Some(desc) = var.description.as_ref() {
            quote! {
                #[doc = #desc]
                #rename
                #v
            }
        } else {
            quote! {
                #rename
                #v
            }
        }
    }
}

pub fn fmt(out_dir: impl AsRef<Path>) {
    use std::io::Write;
    use std::process::Command;
    let out_dir = out_dir.as_ref();
    let dir = std::fs::read_dir(out_dir).unwrap();

    for entry in dir {
        let file = entry.unwrap().file_name().into_string().unwrap();
        if !file.ends_with(".rs") {
            continue;
        }
        let result = Command::new("rustfmt")
            .arg("--emit")
            .arg("files")
            .arg("--edition")
            .arg("2018")
            .arg(out_dir.join(file))
            .output();

        match result {
            Err(e) => {
                eprintln!("error running rustfmt: {e:?}");
            }
            Ok(output) => {
                if !output.status.success() {
                    io::stderr().write_all(&output.stderr).unwrap();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_serde_import() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        Generator::default()
            .out_dir(dir.join("src"))
            .serde(SerdeSupport::with_feature("serde0"))
            .compile_pdls(&[
                dir.join("js_protocol.pdl"),
                dir.join("browser_protocol.pdl"),
            ])
            .unwrap();
    }
}
