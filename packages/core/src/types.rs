//! Core data types for the SemanticWeft protocol.
//!
//! This module defines the wire-format structures that make up a Semantic Unit:
//! [`SemanticUnit`], [`UnitType`], [`RelType`], [`Reference`], and [`Source`].
//! All types serialise to and from JSON exactly as described in
//! `spec/semantic-unit.md`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The semantic role of a unit. Determines how its content is interpreted
/// and which reference relationships are meaningful.
///
/// Serialises as a lowercase snake_case string (e.g. `"assertion"`).
/// See `spec/semantic-unit.md §5` for full type semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnitType {
    /// A claim the author believes to be true.
    Assertion,
    /// An open question the author poses.
    Question,
    /// A claim explicitly derived from one or more other units.
    Inference,
    /// A dispute of another unit's content or assumptions.
    Challenge,
    /// A rule, boundary condition, or invariant others must respect.
    Constraint,
}

/// Formats the type as its lowercase wire-format string (e.g. `"assertion"`).
impl std::fmt::Display for UnitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnitType::Assertion => write!(f, "assertion"),
            UnitType::Question => write!(f, "question"),
            UnitType::Inference => write!(f, "inference"),
            UnitType::Challenge => write!(f, "challenge"),
            UnitType::Constraint => write!(f, "constraint"),
        }
    }
}

/// The typed relationship a referencing unit has to the unit it references.
///
/// Serialises as a kebab-case string (e.g. `"derives-from"`).
/// See `spec/semantic-unit.md §4.4.1` for the full relationship table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RelType {
    /// This unit provides evidence or reasoning for the referenced unit.
    Supports,
    /// This unit argues against the referenced unit's content.
    Rebuts,
    /// This unit's content was logically or causally derived from the referenced unit.
    DerivesFrom,
    /// This unit raises a question about the referenced unit.
    Questions,
    /// This unit narrows, specialises, or adds precision to the referenced unit.
    Refines,
}

/// Formats the relationship as its kebab-case wire-format string (e.g. `"derives-from"`).
impl std::fmt::Display for RelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelType::Supports => write!(f, "supports"),
            RelType::Rebuts => write!(f, "rebuts"),
            RelType::DerivesFrom => write!(f, "derives-from"),
            RelType::Questions => write!(f, "questions"),
            RelType::Refines => write!(f, "refines"),
        }
    }
}

/// Parses a [`UnitType`] from its lowercase wire-format string.
///
/// Returns `Err` with a descriptive message if the string is not recognised.
impl std::str::FromStr for UnitType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "assertion" => Ok(UnitType::Assertion),
            "question" => Ok(UnitType::Question),
            "inference" => Ok(UnitType::Inference),
            "challenge" => Ok(UnitType::Challenge),
            "constraint" => Ok(UnitType::Constraint),
            _ => Err(format!(
                "unknown unit type {:?}; expected one of: \
                 assertion, question, inference, challenge, constraint",
                s
            )),
        }
    }
}

/// Parses a [`RelType`] from its kebab-case wire-format string.
///
/// Returns `Err` with a descriptive message if the string is not recognised.
impl std::str::FromStr for RelType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "supports" => Ok(RelType::Supports),
            "rebuts" => Ok(RelType::Rebuts),
            "derives-from" => Ok(RelType::DerivesFrom),
            "questions" => Ok(RelType::Questions),
            "refines" => Ok(RelType::Refines),
            _ => Err(format!(
                "unknown rel type {:?}; expected one of: \
                 supports, rebuts, derives-from, questions, refines",
                s
            )),
        }
    }
}

/// A typed link from this unit to another unit in the graph.
///
/// Serialises as `{ "id": "<uuidv7>", "rel": "<rel-type>" }`.
/// See `spec/semantic-unit.md §4.4`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    /// UUIDv7 of the referenced unit.
    pub id: String,
    /// The relationship this unit has to the referenced unit.
    pub rel: RelType,
}

/// A citation or provenance reference for a unit's content.
///
/// Serialised as either a plain string (URI or free-form citation) or as a
/// JSON object with `label` and an optional `uri`.
///
/// See `spec/semantic-unit.md §4.3`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Source {
    /// A URI or free-form citation string (e.g. `"Smith et al., 2024"`).
    Uri(String),
    /// A structured citation with a human-readable label and an optional
    /// dereferenceable link.
    Labeled {
        /// Human-readable citation text.
        label: String,
        /// Optional dereferenceable link to the source document.
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
}

/// Controls who may read a unit and how nodes distribute it.
///
/// When absent, `Public` is assumed (backwards-compatible default).
/// Serialises as a lowercase string. See `spec/semantic-unit.md §4.5`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Readable by anyone; enters global sync. Default.
    #[default]
    Public,
    /// Delivered only to agents that follow the author.
    Network,
    /// Delivered only to agents listed in `audience`.
    Limited,
}

/// Formats the visibility as its lowercase wire-format string.
impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Network => write!(f, "network"),
            Visibility::Limited => write!(f, "limited"),
        }
    }
}

/// Parses a [`Visibility`] from its lowercase wire-format string.
///
/// Returns `Err` with a descriptive message if the string is not recognised.
impl std::str::FromStr for Visibility {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Visibility::Public),
            "network" => Ok(Visibility::Network),
            "limited" => Ok(Visibility::Limited),
            _ => Err(format!(
                "unknown visibility {:?}; expected one of: public, network, limited",
                s
            )),
        }
    }
}

/// A Semantic Unit — the fundamental record type of the SemanticWeft protocol.
///
/// Units are immutable once created. An agent wishing to revise a unit creates
/// a new unit referencing the original rather than modifying it.
///
/// # Creation
///
/// Use [`SemanticUnit::new`] for convenience, or construct directly and call
/// [`validate_unit`](crate::validate_unit) to verify conformance.
///
/// # Extension fields
///
/// Fields not defined by the spec are captured in `extensions`. Their names
/// must match `^x-[a-z0-9]+(\.[a-z0-9]+)+$` to be valid.
///
/// See `spec/semantic-unit.md §3–4` for the full field specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticUnit {
    /// UUIDv7 identifier (RFC 9562). The time component encodes creation order.
    pub id: String,

    /// The semantic role of this unit.
    #[serde(rename = "type")]
    pub unit_type: UnitType,

    /// The human-readable statement this unit expresses. Must be non-empty.
    pub content: String,

    /// ISO 8601 timestamp of creation. SHOULD be UTC.
    pub created_at: String,

    /// Identifier of the agent that created this unit. Must be non-empty.
    pub author: String,

    /// Degree of belief: 0.0 (none) to 1.0 (certain). Absence ≠ 0.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,

    /// Conditions that must hold for this unit's content to be valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumptions: Option<Vec<String>>,

    /// Citation or provenance for this unit's content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,

    /// Typed links to other units this unit relates to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<Reference>>,

    /// Access control for this unit. Absent means `Public`.
    /// Immutable once set. See `spec/semantic-unit.md §4.5`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,

    /// Recipients permitted to read this unit.
    /// Required when `visibility` is `Limited`; must be absent otherwise.
    /// See `spec/semantic-unit.md §4.6`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,

    /// Extension fields (`x-<reverse-domain>.<name>`).
    ///
    /// Captured via `#[serde(flatten)]` so they round-trip cleanly through
    /// JSON serialisation without any explicit enumeration.
    #[serde(flatten)]
    pub extensions: HashMap<String, serde_json::Value>,
}

impl SemanticUnit {
    /// Create a new unit with auto-generated UUIDv7 `id` and current UTC `created_at`.
    ///
    /// The returned unit is structurally valid. Call [`validate_unit`](crate::validate_unit)
    /// after setting optional fields if you want full conformance checking.
    pub fn new(
        unit_type: UnitType,
        content: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            unit_type,
            content: content.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            author: author.into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: None,
            visibility: None,
            audience: None,
            extensions: HashMap::new(),
        }
    }
}
