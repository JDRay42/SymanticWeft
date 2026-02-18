use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The semantic role of a unit. Determines how its content is interpreted
/// and which reference relationships are meaningful.
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

/// A typed link from this unit to another unit in the graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    /// UUIDv7 of the referenced unit.
    pub id: String,
    /// The relationship this unit has to the referenced unit.
    pub rel: RelType,
}

/// A citation or provenance reference for a unit's content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Source {
    /// A URI or free-form citation string.
    Uri(String),
    /// A structured citation with a human-readable label and an optional URI.
    Labeled {
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticUnit {
    /// UUIDv7 identifier (RFC 9562). The time component encodes creation order.
    pub id: String,

    /// The semantic role of this unit.
    #[serde(rename = "type")]
    pub unit_type: UnitType,

    /// The human-readable statement this unit expresses.
    pub content: String,

    /// ISO 8601 timestamp of creation. SHOULD be UTC.
    pub created_at: String,

    /// Identifier of the agent that created this unit.
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

    /// Extension fields (`x-<reverse-domain>.<name>`).
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
            extensions: HashMap::new(),
        }
    }
}
