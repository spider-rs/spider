use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::pdl::{DataType, Domain, Event};

pub struct EventType<'a> {
    pub protocol_mod: Ident,
    pub domain: &'a Domain<'a>,
    pub inner: &'a Event<'a>,
    pub needs_box: bool,
}

impl<'a> EventType<'a> {
    fn ty_ident(&self) -> Ident {
        format_ident!("Event{}", self.inner.name().to_upper_camel_case())
    }

    fn var_ident(&self) -> Ident {
        format_ident!(
            "{}{}",
            self.domain.name.to_upper_camel_case(),
            self.inner.name().to_upper_camel_case()
        )
    }
}

pub struct EventBuilder<'a> {
    events: Vec<EventType<'a>>,
}

impl<'a> EventBuilder<'a> {
    pub fn new(events: Vec<EventType<'a>>) -> Self {
        Self { events }
    }

    pub fn build(self) -> TokenStream {
        let mut variants_stream = TokenStream::default();
        let mut var_idents = Vec::new();
        let mut deserialize_from_method = TokenStream::default();
        let mut conversion_impls = TokenStream::default();
        let mut event_trait_impls = TokenStream::default();
        let mut consume_event_macro_exprs = TokenStream::default();
        let mut event_as_boxed_results = TokenStream::default();

        for event in &self.events {
            let var_ident = event.var_ident();

            let ty_ident = event.ty_ident();

            let deprecated = if event.inner.is_deprecated() {
                quote! {[deprecated]}
            } else {
                TokenStream::default()
            };

            let domain_mod = format_ident!("{}", event.domain.name.to_snake_case());
            let protocol_mod = &event.protocol_mod;

            let ty_qualifier = quote! {super::#protocol_mod::#domain_mod::#ty_ident};

            let ty_ident = if event.needs_box {
                quote! {Box<#ty_qualifier>}
            } else {
                ty_qualifier.clone()
            };

            variants_stream.extend(quote! {
                #deprecated
                #var_ident(#ty_ident),
            });

            let (variant_match, into_event, consume_event_macro_expr, event_as_boxed_result) =
                if event.needs_box {
                    (
                        quote! {
                            CdpEvent::#var_ident(val) => Ok(*val),
                        },
                        quote! {
                            CdpEvent::#var_ident(Box::new(el))
                        },
                        quote! {
                            CdpEvent::#var_ident(event) => {$builtin(*event);}
                        },
                        quote! {
                            CdpEvent::#var_ident(event) => Ok(Box::new(*event)),
                        },
                    )
                } else {
                    (
                        quote! {
                            CdpEvent::#var_ident(val) => Ok(val),
                        },
                        quote! {
                            CdpEvent::#var_ident(el)
                        },
                        quote! {
                            CdpEvent::#var_ident(event) => {$builtin(event);}
                        },
                        quote! {
                            CdpEvent::#var_ident(event) => Ok(Box::new(event)),
                        },
                    )
                };

            event_as_boxed_results.extend(event_as_boxed_result);

            consume_event_macro_exprs.extend(consume_event_macro_expr);

            conversion_impls.extend(quote! {
                impl std::convert::TryFrom<CdpEvent> for  #ty_qualifier {
                    type Error = CdpEvent;

                    fn try_from(event: CdpEvent) -> Result<Self, Self::Error> {
                        match event {
                            #variant_match
                            _ => Err(event)
                        }
                    }
                }
                impl From<#ty_qualifier> for CdpEvent {
                    fn from(el: #ty_qualifier) -> CdpEvent {
                        #into_event
                    }
                }
            });

            event_trait_impls.extend(quote! {
                    impl super::sealed::SealedEvent for #ty_qualifier {
                        fn as_any(&self) -> &dyn ::std::any::Any {
                            self
                        }
                    }
                     impl super::IntoEventKind for #ty_qualifier {

                       fn event_kind() -> super::EventKind where Self: Sized + 'static  {
                           super::EventKind::BuiltIn
                       }
                     }

            });

            let deserialize_from = if event.needs_box {
                quote! {
                        #ty_qualifier::IDENTIFIER =>CdpEvent::#var_ident(Box::new(map.next_value::<#ty_qualifier>()?)),
                }
            } else {
                quote! {
                        #ty_qualifier::IDENTIFIER =>CdpEvent::#var_ident(map.next_value::<#ty_qualifier>()?),
                }
            };

            deserialize_from_method.extend(deserialize_from);

            var_idents.push(var_ident);
        }

        let event_impl = quote! {
            #[derive(Debug, PartialEq, Clone)]
            pub struct CdpEventMessage {
                /// Name of the method
                pub method: chromiumoxide_types::MethodId,
                /// The chromium session Id
                pub session_id: Option<String>,
                /// Json params
                pub params: CdpEvent,
            }
            impl chromiumoxide_types::Method for CdpEventMessage {
                fn identifier(&self) -> chromiumoxide_types::MethodId {
                   match &self.params {
                        #(CdpEvent::#var_idents(inner) => inner.identifier(),)*
                        _=> self.method.clone()
                    }
                }
            }
            impl chromiumoxide_types::EventMessage for CdpEventMessage {
                fn session_id(&self) -> Option<&str> {
                    self.session_id.as_deref()
                }
            }

            #[derive(Debug, Clone, PartialEq)]
            pub enum CdpEvent {
                #variants_stream
                Other(serde_json::Value)
            }

            impl CdpEvent {

                pub fn other(other: serde_json::Value) -> Self {
                    CdpEvent::Other(other)
                }

                /// Serializes the event as Json
                pub fn into_json(self) -> serde_json::Result<serde_json::Value> {
                    match self {
                        #(CdpEvent::#var_idents(inner) => serde_json::to_value(inner),)*
                         CdpEvent::Other(val) => Ok(val)
                    }
                }

                pub fn into_event(self) -> ::std::result::Result<Box<dyn super::Event>, serde_json::Value> {
                    match self {
                        #event_as_boxed_results
                        CdpEvent::Other(other) => Err(other)
                    }
                }

           }
           // #event_json serde.generate_event_json_support
        };

        let deserialize_impl = quote! {
            use std::fmt;
            use serde::de::{self, Deserializer, MapAccess, Visitor};
            impl<'de> Deserialize<'de> for CdpEventMessage {
                fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
                where
                    D: Deserializer<'de>,
                {
                    enum Field {
                        Method,
                        Session,
                        Params,
                    }

                    impl<'de> Deserialize<'de> for Field {
                        fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
                        where
                            D: Deserializer<'de>,
                        {
                            struct FieldVisitor;

                            impl<'de> Visitor<'de> for FieldVisitor {
                                type Value = Field;

                                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                                    formatter.write_str("`method` or `sessionId` or `params`")
                                }

                                fn visit_str<E>(self, value: &str) -> Result<Field, E>
                                where
                                    E: de::Error,
                                {
                                    match value {
                                        "method" => Ok(Field::Method),
                                        "sessionId" => Ok(Field::Session),
                                        "params" => Ok(Field::Params),
                                        _ => Err(de::Error::unknown_field(value, FIELDS)),
                                    }
                                }
                            }

                            deserializer.deserialize_identifier(FieldVisitor)
                        }
                    }

                    struct MessageVisitor;

                    impl<'de> Visitor<'de> for MessageVisitor {
                        type Value = CdpEventMessage;

                        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                            formatter.write_str("struct CdpEventMessage")
                        }

                        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                        where
                            A: MapAccess<'de>,
                        {
                            let mut method = None;
                            let mut session_id = None;
                            let mut params = None;
                            while let Some(key) = map.next_key()? {
                                match key {
                                    Field::Method => {
                                        if method.is_some() {
                                            return Err(de::Error::duplicate_field("method"));
                                        }
                                        method = Some(map.next_value::<String>()?);
                                    }
                                    Field::Session => {
                                        if session_id.is_some() {
                                            return Err(de::Error::duplicate_field("sessionId"));
                                        }
                                        session_id = Some(map.next_value::<String>()?);
                                    }
                                    Field::Params => {
                                        if params.is_some() {
                                            return Err(de::Error::duplicate_field("params"));
                                        }
                                        params = Some(match method.as_ref().ok_or_else(|| de::Error::missing_field("params"))
                                        ?.as_str() {
                                            #deserialize_from_method
                                            _=>CdpEvent::Other(map.next_value::<serde_json::Value>()?)
                                        });
                                    }
                                }
                            }

                            let method = method.ok_or_else(|| de::Error::missing_field("method"))?;
                            let params = params.ok_or_else(|| de::Error::missing_field("params"))?;
                            Ok(CdpEventMessage {
                                method: ::std::borrow::Cow::Owned(method),
                                session_id,
                                params
                            })
                        }
                    }
                    const FIELDS: &[&str] = &["method", "sessionId", "params"];
                    deserializer.deserialize_struct("CdpEventMessage", FIELDS, MessageVisitor)
                }
            }

            impl std::convert::TryInto<chromiumoxide_types::CdpJsonEventMessage> for CdpEventMessage {
                type Error = serde_json::Error;

                fn try_into(self) -> Result<chromiumoxide_types::CdpJsonEventMessage, Self::Error> {
                    use chromiumoxide_types::Method;
                    Ok(chromiumoxide_types::CdpJsonEventMessage {
                        method: self.identifier(),
                        session_id: self.session_id,
                        params: self.params.into_json()?
                    })
                }
           }

        };

        quote! {
            #event_impl
            #deserialize_impl
            #conversion_impls
            #event_trait_impls

            #[macro_export]
            #[doc(hidden)]
            macro_rules! consume_event {
                (match $ev:ident  { $builtin:expr, $custom: expr  }) => {
                    {
                        match $ev {
                           #consume_event_macro_exprs
                           CdpEvent::Other(json) => {$custom(json);}
                        }
                    }
                };
            }
        }
    }
}
