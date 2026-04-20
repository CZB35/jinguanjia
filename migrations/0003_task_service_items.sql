-- 多服务环节任务子表

SET NAMES utf8mb4;

CREATE TABLE IF NOT EXISTS `task_service_item` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `service_type` VARCHAR(30) NOT NULL COMMENT '服务环节: measure/install/ironing/cleaning',
  `service_status` VARCHAR(30) NOT NULL DEFAULT 'pending_assign' COMMENT '环节状态',
  `technician_id` BIGINT NULL COMMENT '环节技师ID',
  `scheduled_time` DATETIME NULL COMMENT '环节预约时间',
  `started_at` DATETIME NULL COMMENT '环节开始时间',
  `completed_at` DATETIME NULL COMMENT '环节完成时间',
  `progress_percent` INT NOT NULL DEFAULT 0 COMMENT '环节进度百分比',
  `sort_no` INT NOT NULL DEFAULT 1 COMMENT '排序',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  UNIQUE KEY `uk_task_service_item_task_service` (`task_id`, `service_type`),
  KEY `idx_task_service_item_task_id` (`task_id`),
  KEY `idx_task_service_item_service_type_status` (`service_type`, `service_status`),
  KEY `idx_task_service_item_technician_id` (`technician_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='任务服务环节子表';

INSERT INTO `task_service_item` (`task_id`, `service_type`, `service_status`, `technician_id`, `scheduled_time`, `started_at`, `completed_at`, `progress_percent`, `sort_no`)
SELECT
  t.`id`,
  t.`service_type`,
  t.`task_status`,
  t.`current_technician_id`,
  t.`appointment_time`,
  te.`started_at`,
  te.`submitted_at`,
  CASE
    WHEN t.`task_status` = 'executing' THEN 60
    WHEN t.`task_status` = 'pending_dealer_review' THEN 90
    WHEN t.`task_status` = 'pending_hq_score' THEN 95
    WHEN t.`task_status` IN ('completed', 'closed') THEN 100
    ELSE 0
  END,
  1
FROM `task` t
LEFT JOIN `task_execution` te ON te.`id` = t.`latest_execution_id`
WHERE t.`service_type` IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM `task_service_item` tsi WHERE tsi.`task_id` = t.`id` AND tsi.`service_type` = t.`service_type`
  );
