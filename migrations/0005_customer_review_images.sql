-- 客户评价图片表

SET NAMES utf8mb4;

CREATE TABLE IF NOT EXISTS `customer_review_image` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `review_link_id` BIGINT NOT NULL COMMENT '评价链接ID',
  `review_id` BIGINT NULL COMMENT '评价ID',
  `file_name` VARCHAR(255) NOT NULL COMMENT '文件名',
  `file_url` VARCHAR(500) NOT NULL COMMENT '文件地址',
  `file_size` BIGINT NOT NULL COMMENT '文件大小',
  `mime_type` VARCHAR(100) NOT NULL COMMENT 'MIME类型',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_customer_review_image_link_id` (`review_link_id`),
  KEY `idx_customer_review_image_review_id` (`review_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户评价图片表';
