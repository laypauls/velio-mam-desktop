//! `SearchFilters.userFields`(프론트 형태) -> `mam_search::UserFieldCondition` 변환.
//! 현재 정의와의 대조(없는·삭제 중인 필드/선택지, 타입 불일치)는 `QueryService::validate_user_fields` 가 먼저 한다.

use mam_core::query::UserFieldFilter;
use mam_core::scan::IngestError;
use mam_search::UserFieldCondition;

fn id(v: i64) -> Result<u64, IngestError> {
    u64::try_from(v).map_err(|_| IngestError::InvalidInput("userFields"))
}

fn ids(v: &[i64]) -> Result<Vec<u64>, IngestError> {
    v.iter().map(|x| id(*x)).collect()
}

pub(crate) fn to_conditions(
    src: &[UserFieldFilter],
) -> Result<Vec<UserFieldCondition>, IngestError> {
    src.iter()
        .map(|c| {
            Ok(match c {
                UserFieldFilter::KeywordAnyOf { field_id, values } => {
                    UserFieldCondition::KeywordAnyOf {
                        field_id: id(*field_id)?,
                        values: values.clone(),
                    }
                }
                UserFieldFilter::NumberRange { field_id, min, max } => {
                    UserFieldCondition::NumberRange {
                        field_id: id(*field_id)?,
                        min: *min,
                        max: *max,
                    }
                }
                UserFieldFilter::DateRange { field_id, min, max } => {
                    UserFieldCondition::DateRange {
                        field_id: id(*field_id)?,
                        min: *min,
                        max: *max,
                    }
                }
                UserFieldFilter::Bool { field_id, value } => UserFieldCondition::Bool {
                    field_id: id(*field_id)?,
                    value: *value,
                },
                UserFieldFilter::EnumAnyOf {
                    field_id,
                    option_ids,
                } => UserFieldCondition::EnumAnyOf {
                    field_id: id(*field_id)?,
                    option_ids: ids(option_ids)?,
                },
                UserFieldFilter::MultiEnum {
                    field_id,
                    option_ids,
                    all,
                } => UserFieldCondition::MultiEnum {
                    field_id: id(*field_id)?,
                    option_ids: ids(option_ids)?,
                    all: *all,
                },
                UserFieldFilter::Has { field_id } => UserFieldCondition::Has {
                    field_id: id(*field_id)?,
                },
                UserFieldFilter::Missing { field_id } => UserFieldCondition::Missing {
                    field_id: id(*field_id)?,
                },
            })
        })
        .collect()
}
