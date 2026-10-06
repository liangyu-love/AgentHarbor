use super::{projection::ResolvedMember, types::AggregateMember};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub member_id: String,
    pub provider_id: String,
    pub actual_model_id: String,
    pub priority: u16,
    pub weight: u16,
    /// Hash of actual endpoint, credential, full wire model and effective protocol.
    pub target_key: String,
}

impl Candidate {
    pub fn from_member(member: &AggregateMember, target_key: String) -> Self {
        Self {
            member_id: member.id.clone(),
            provider_id: member.provider_id.clone(),
            actual_model_id: member.actual_model_id.clone(),
            priority: member.priority,
            weight: member.weight,
            target_key,
        }
    }
}

impl From<&ResolvedMember> for Candidate {
    fn from(member: &ResolvedMember) -> Self {
        Self::from_member(&member.member, member.target_key.clone())
    }
}
