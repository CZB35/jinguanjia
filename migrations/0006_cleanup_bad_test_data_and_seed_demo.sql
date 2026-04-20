-- 清理写坏的测试数据并插入正常中文演示数据

SET NAMES utf8mb4;

DELETE cri
FROM `customer_review_image` cri
LEFT JOIN `customer_review_link` crl ON crl.id = cri.review_link_id
LEFT JOIN `customer_review` cr ON cr.id = cri.review_id
WHERE crl.task_id IN (3,4,5,6,7,8,9,10)
   OR cr.task_id IN (3,4,5,6,7,8,9,10)
   OR crl.customer_id IN (SELECT id FROM `customer` WHERE mobile IN ('13912345678','13999990000','13999990001'));

DELETE FROM `customer_review`
WHERE task_id IN (3,4,5,6,7,8,9,10)
   OR customer_id IN (SELECT id FROM `customer` WHERE mobile IN ('13912345678','13999990000','13999990001'));

DELETE FROM `customer_review_link`
WHERE task_id IN (3,4,5,6,7,8,9,10)
   OR customer_id IN (SELECT id FROM `customer` WHERE mobile IN ('13912345678','13999990000','13999990001'));

DELETE FROM `dealer_review` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `hq_score_item` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `hq_score` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `evidence` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `attachment` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `task_execution` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `task_assignment` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `task_service_item` WHERE task_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `audit_log` WHERE biz_type = 'task' AND biz_id IN (3,4,5,6,7,8,9,10);
DELETE FROM `notice` WHERE receiver_id IN (4,5,8,9,12,13,16,17,18,19,20,21);
DELETE FROM `task` WHERE id IN (3,4,5,6,7,8,9,10);
DELETE FROM `customer_address` WHERE customer_id IN (SELECT id FROM `customer` WHERE mobile IN ('13912345678','13999990000','13999990001'));
DELETE FROM `customer` WHERE mobile IN ('13912345678','13999990000','13999990001');
DELETE FROM `technician_skill` WHERE user_id IN (4,5,8,9,12,13,16,17,18,19,20,21);
DELETE FROM `user_message_setting` WHERE user_id IN (4,5,8,9,12,13,16,17,18,19,20,21);
DELETE FROM `user_role` WHERE user_id IN (4,5,8,9,12,13,16,17,18,19,20,21);
DELETE FROM `user_store` WHERE user_id IN (4,5,8,9,12,13,16,17,18,19,20,21);
DELETE FROM `user` WHERE id IN (4,5,8,9,12,13,16,17,18,19,20,21);

INSERT INTO `user` (`mobile`, `password_hash`, `name`, `nickname`, `email`, `job_no`, `status`)
SELECT '13600000001', '$2b$10$0pAAaLAPE6GcIT7m4yK6muqLff6fnoBNdkyoW/muQ8wHduSi7HP5.', '陈店长', '陈店长', 'dealer-demo@jinguanjia.local', 'DL-DEALER-001', 'enabled'
WHERE NOT EXISTS (SELECT 1 FROM `user` WHERE mobile = '13600000001');

INSERT INTO `user` (`mobile`, `password_hash`, `name`, `nickname`, `email`, `job_no`, `status`, `emergency_contact_name`, `emergency_contact_mobile`)
SELECT '13600000002', '$2b$10$0pAAaLAPE6GcIT7m4yK6muqLff6fnoBNdkyoW/muQ8wHduSi7HP5.', '王师傅', '老王', 'technician-demo@jinguanjia.local', 'DL-20260315', 'enabled', '李女士', '13600009999'
WHERE NOT EXISTS (SELECT 1 FROM `user` WHERE mobile = '13600000002');

INSERT INTO `user_role` (`user_id`, `role_id`)
SELECT u.id, r.id
FROM `user` u JOIN `role` r ON r.role_code = 'dealer'
WHERE u.mobile = '13600000001'
  AND NOT EXISTS (SELECT 1 FROM `user_role` ur WHERE ur.user_id = u.id AND ur.role_id = r.id);

INSERT INTO `user_role` (`user_id`, `role_id`)
SELECT u.id, r.id
FROM `user` u JOIN `role` r ON r.role_code = 'technician'
WHERE u.mobile = '13600000002'
  AND NOT EXISTS (SELECT 1 FROM `user_role` ur WHERE ur.user_id = u.id AND ur.role_id = r.id);

INSERT INTO `user_store` (`user_id`, `store_id`)
SELECT u.id, 1 FROM `user` u
WHERE u.mobile IN ('13600000001','13600000002')
  AND NOT EXISTS (SELECT 1 FROM `user_store` us WHERE us.user_id = u.id AND us.store_id = 1);

INSERT INTO `technician_skill` (`user_id`, `service_type`)
SELECT u.id, 'install' FROM `user` u
WHERE u.mobile = '13600000002'
  AND NOT EXISTS (SELECT 1 FROM `technician_skill` ts WHERE ts.user_id = u.id AND ts.service_type = 'install');

INSERT INTO `technician_skill` (`user_id`, `service_type`)
SELECT u.id, 'cleaning' FROM `user` u
WHERE u.mobile = '13600000002'
  AND NOT EXISTS (SELECT 1 FROM `technician_skill` ts WHERE ts.user_id = u.id AND ts.service_type = 'cleaning');

INSERT INTO `user_message_setting` (`user_id`, `new_task_enabled`, `review_reminder_enabled`, `sms_enabled`, `wechat_enabled`)
SELECT u.id, 1, 1, 1, 1
FROM `user` u
WHERE u.mobile IN ('13600000001','13600000002')
  AND NOT EXISTS (SELECT 1 FROM `user_message_setting` ums WHERE ums.user_id = u.id);

INSERT INTO `customer` (`name`, `mobile`, `source`, `remark`, `created_by`)
SELECT '张先生', '13900000001', 'manual', '演示客户一', u.id
FROM `user` u
WHERE u.mobile = '13600000001'
  AND NOT EXISTS (SELECT 1 FROM `customer` c WHERE c.mobile = '13900000001');

INSERT INTO `customer` (`name`, `mobile`, `source`, `remark`, `created_by`)
SELECT '李女士', '13900000002', 'manual', '演示客户二', u.id
FROM `user` u
WHERE u.mobile = '13600000001'
  AND NOT EXISTS (SELECT 1 FROM `customer` c WHERE c.mobile = '13900000002');

INSERT INTO `customer_address` (`customer_id`, `province`, `city`, `district`, `address`, `is_default`)
SELECT c.id, '浙江省', '杭州市', '西湖区', '文三路100号', 1
FROM `customer` c
WHERE c.mobile = '13900000001'
  AND NOT EXISTS (SELECT 1 FROM `customer_address` ca WHERE ca.customer_id = c.id AND ca.address = '文三路100号');

INSERT INTO `customer_address` (`customer_id`, `province`, `city`, `district`, `address`, `is_default`)
SELECT c.id, '浙江省', '杭州市', '拱墅区', '湖州街88号', 1
FROM `customer` c
WHERE c.mobile = '13900000002'
  AND NOT EXISTS (SELECT 1 FROM `customer_address` ca WHERE ca.customer_id = c.id AND ca.address = '湖州街88号');

INSERT INTO `task` (`task_no`, `customer_id`, `customer_address_id`, `store_id`, `dealer_user_id`, `current_technician_id`, `service_type`, `priority`, `appointment_time`, `service_time_slot`, `task_desc`, `task_remark`, `task_status`, `review_status`, `score_status`, `is_overdue`, `source`, `created_by`)
SELECT 'DEMO202604190001', c.id, ca.id, 1, du.id, tu.id, 'install', 'normal', '2026-04-20 14:30:00', '下午', '窗帘安装与除尘除螨', '演示已完成任务', 'completed', 'reviewed', 'scored', 0, 'mini_program', du.id
FROM `customer` c
JOIN `customer_address` ca ON ca.customer_id = c.id AND ca.is_default = 1
JOIN `user` du ON du.mobile = '13600000001'
JOIN `user` tu ON tu.mobile = '13600000002'
WHERE c.mobile = '13900000001'
  AND NOT EXISTS (SELECT 1 FROM `task` t WHERE t.task_no = 'DEMO202604190001');

INSERT INTO `task` (`task_no`, `customer_id`, `customer_address_id`, `store_id`, `dealer_user_id`, `current_technician_id`, `service_type`, `priority`, `appointment_time`, `service_time_slot`, `task_desc`, `task_remark`, `task_status`, `review_status`, `score_status`, `is_overdue`, `source`, `created_by`)
SELECT 'DEMO202604190002', c.id, ca.id, 1, du.id, tu.id, 'install', 'high', '2026-04-21 10:00:00', '上午', '窗帘安装服务', '演示待执行任务', 'pending_execute', 'pending', 'pending', 0, 'mini_program', du.id
FROM `customer` c
JOIN `customer_address` ca ON ca.customer_id = c.id AND ca.is_default = 1
JOIN `user` du ON du.mobile = '13600000001'
JOIN `user` tu ON tu.mobile = '13600000002'
WHERE c.mobile = '13900000002'
  AND NOT EXISTS (SELECT 1 FROM `task` t WHERE t.task_no = 'DEMO202604190002');

INSERT INTO `task_assignment` (`task_id`, `from_technician_id`, `to_technician_id`, `assign_type`, `assign_reason`, `assigned_by`, `assigned_at`)
SELECT t.id, NULL, tu.id, 'assign', '演示任务分派', du.id, '2026-04-19 10:00:00'
FROM `task` t JOIN `user` du ON du.mobile='13600000001' JOIN `user` tu ON tu.mobile='13600000002'
WHERE t.task_no IN ('DEMO202604190001','DEMO202604190002')
  AND NOT EXISTS (SELECT 1 FROM `task_assignment` ta WHERE ta.task_id = t.id);

INSERT INTO `task_execution` (`task_id`, `technician_id`, `started_at`, `submitted_at`, `submit_remark`, `execution_status`)
SELECT t.id, tu.id, '2026-04-19 10:10:00', '2026-04-19 10:30:00', '演示提交', 'submitted'
FROM `task` t JOIN `user` tu ON tu.mobile='13600000002'
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `task_execution` te WHERE te.task_id = t.id);

INSERT INTO `task_service_item` (`task_id`, `service_type`, `service_status`, `technician_id`, `scheduled_time`, `started_at`, `completed_at`, `progress_percent`, `sort_no`)
SELECT t.id, 'install', 'completed', tu.id, '2026-04-20 14:30:00', '2026-04-19 10:10:00', '2026-04-19 10:25:00', 100, 1
FROM `task` t JOIN `user` tu ON tu.mobile='13600000002'
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `task_service_item` tsi WHERE tsi.task_id=t.id AND tsi.service_type='install');

INSERT INTO `task_service_item` (`task_id`, `service_type`, `service_status`, `technician_id`, `scheduled_time`, `started_at`, `completed_at`, `progress_percent`, `sort_no`)
SELECT t.id, 'cleaning', 'completed', tu.id, '2026-04-20 14:30:00', '2026-04-19 10:15:00', '2026-04-19 10:30:00', 100, 2
FROM `task` t JOIN `user` tu ON tu.mobile='13600000002'
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `task_service_item` tsi WHERE tsi.task_id=t.id AND tsi.service_type='cleaning');

INSERT INTO `task_service_item` (`task_id`, `service_type`, `service_status`, `technician_id`, `scheduled_time`, `progress_percent`, `sort_no`)
SELECT t.id, 'install', 'pending_execute', tu.id, '2026-04-21 10:00:00', 0, 1
FROM `task` t JOIN `user` tu ON tu.mobile='13600000002'
WHERE t.task_no='DEMO202604190002'
  AND NOT EXISTS (SELECT 1 FROM `task_service_item` tsi WHERE tsi.task_id=t.id AND tsi.service_type='install');

INSERT INTO `dealer_review` (`task_id`, `reviewer_id`, `star`, `tag_json`, `content`)
SELECT t.id, du.id, 5, JSON_ARRAY('服务专业','准时上门'), '本次服务非常满意，安装规范，准时上门。'
FROM `task` t JOIN `user` du ON du.mobile='13600000001'
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `dealer_review` dr WHERE dr.task_id=t.id);

INSERT INTO `hq_score` (`task_id`, `scorer_id`, `total_score`, `service_level`, `score_remark`)
SELECT t.id, su.id, 90, 'excellent', '总部检核通过，服务质量优秀。'
FROM `task` t JOIN `user` su ON su.mobile='13800000000'
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `hq_score` hs WHERE hs.task_id=t.id);

INSERT INTO `hq_score_item` (`hq_score_id`, `task_id`, `service_type`, `score`, `deduction_reason`)
SELECT hs.id, t.id, 'install', 92, ''
FROM `task` t JOIN `hq_score` hs ON hs.task_id=t.id
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `hq_score_item` hsi WHERE hsi.task_id=t.id AND hsi.service_type='install');

INSERT INTO `hq_score_item` (`hq_score_id`, `task_id`, `service_type`, `score`, `deduction_reason`)
SELECT hs.id, t.id, 'cleaning', 88, ''
FROM `task` t JOIN `hq_score` hs ON hs.task_id=t.id
WHERE t.task_no='DEMO202604190001'
  AND NOT EXISTS (SELECT 1 FROM `hq_score_item` hsi WHERE hsi.task_id=t.id AND hsi.service_type='cleaning');

UPDATE `task` t
SET t.latest_assignment_id = (SELECT ta.id FROM `task_assignment` ta WHERE ta.task_id=t.id ORDER BY ta.id DESC LIMIT 1),
    t.latest_execution_id = (SELECT te.id FROM `task_execution` te WHERE te.task_id=t.id ORDER BY te.id DESC LIMIT 1)
WHERE t.task_no IN ('DEMO202604190001','DEMO202604190002');
