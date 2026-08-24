use std::collections::BTreeSet;

use chrono::NaiveDateTime;

use crate::models::{
    evidence::EvidenceEntity, task_execution::TaskExecutionEntity,
    task_service_item::TaskServiceItemEntity,
};

pub fn latest_technician_location_address(
    evidences: &[EvidenceEntity],
    executions: &[TaskExecutionEntity],
    service_items: &[TaskServiceItemEntity],
    customer_address: &str,
) -> String {
    let scoped_execution_ids = scoped_execution_ids(executions, service_items);
    evidences
        .iter()
        .filter(|evidence| {
            scoped_execution_ids
                .as_ref()
                .map(|execution_ids| {
                    evidence
                        .execution_id
                        .map(|id| execution_ids.contains(&id))
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .filter_map(|evidence| {
            evidence_location_address(evidence, customer_address).map(|address| {
                (
                    evidence_sort_time(evidence),
                    evidence.id.unwrap_or_default(),
                    address,
                )
            })
        })
        .max_by_key(|(sort_time, id, _)| (*sort_time, *id))
        .map(|(_, _, address)| address)
        .unwrap_or_default()
}

fn scoped_execution_ids(
    executions: &[TaskExecutionEntity],
    service_items: &[TaskServiceItemEntity],
) -> Option<BTreeSet<i64>> {
    if service_items.is_empty() {
        return None;
    }

    let item_ids = service_items
        .iter()
        .filter_map(|item| item.id)
        .collect::<BTreeSet<_>>();
    let service_types = service_items
        .iter()
        .filter_map(|item| item.service_type.as_deref())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();

    Some(
        executions
            .iter()
            .filter(|execution| execution.execution_status.as_deref() != Some("cancelled"))
            .filter(|execution| {
                execution
                    .service_item_id
                    .map(|id| item_ids.contains(&id))
                    .unwrap_or_else(|| {
                        item_ids.is_empty()
                            && execution
                                .service_type
                                .as_deref()
                                .map(|service_type| service_types.contains(service_type))
                                .unwrap_or(false)
                    })
            })
            .filter_map(|execution| execution.id)
            .collect(),
    )
}

fn evidence_location_address(evidence: &EvidenceEntity, customer_address: &str) -> Option<String> {
    let customer_address = customer_address.trim();
    if let Some(address) = evidence
        .location_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty() && *address != customer_address)
    {
        return Some(address.to_string());
    }

    parse_technician_location_from_watermark(evidence.watermark_text.as_deref(), customer_address)
}

fn evidence_sort_time(evidence: &EvidenceEntity) -> Option<NaiveDateTime> {
    evidence.shot_at.or(evidence.created_at)
}

fn parse_technician_location_from_watermark(
    watermark_text: Option<&str>,
    customer_address: &str,
) -> Option<String> {
    let text = watermark_text?;
    let address = extract_between(text, "地址:", " 客户地址:")?;
    let customer = extract_between(text, "客户地址:", " 客户:").unwrap_or_default();
    let address = address.trim();
    let customer = customer.trim();
    if address.is_empty()
        || address == "-"
        || address == customer_address.trim()
        || (!customer.is_empty() && address == customer)
    {
        None
    } else {
        Some(address.to_string())
    }
}

fn extract_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let (_, after_start) = text.split_once(start)?;
    let (value, _) = after_start.split_once(end)?;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watermark_location_ignores_customer_fallback() {
        let text = "门店:A 地址:河北省石家庄市长安区123 客户地址:河北省石家庄市长安区123 客户:张三(13800000000) 服务:量尺 时间:2026-06-22 08:49";
        assert_eq!(
            parse_technician_location_from_watermark(Some(text), "河北省石家庄市长安区123"),
            None
        );
    }

    #[test]
    fn watermark_location_keeps_real_technician_location() {
        let text = "门店:A 地址:临颍县荣耀授权专卖店 客户地址:河北省石家庄市长安区123 客户:张三(13800000000) 服务:量尺 时间:2026-06-22 08:49";
        assert_eq!(
            parse_technician_location_from_watermark(Some(text), "河北省石家庄市长安区123")
                .as_deref(),
            Some("临颍县荣耀授权专卖店")
        );
    }

    #[test]
    fn scoped_location_ignores_detached_historical_execution() {
        let service_items = vec![TaskServiceItemEntity {
            id: Some(101),
            service_type: Some("install".to_string()),
            ..Default::default()
        }];
        let executions = vec![
            TaskExecutionEntity {
                id: Some(201),
                service_item_id: None,
                service_type: Some("install".to_string()),
                ..Default::default()
            },
            TaskExecutionEntity {
                id: Some(202),
                service_item_id: Some(101),
                service_type: Some("install".to_string()),
                execution_status: Some("cancelled".to_string()),
                ..Default::default()
            },
        ];

        assert_eq!(
            scoped_execution_ids(&executions, &service_items),
            Some(BTreeSet::new())
        );
        assert_eq!(scoped_execution_ids(&executions, &[]), None);
    }
}
