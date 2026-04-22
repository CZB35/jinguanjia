SET NAMES utf8mb4;

INSERT INTO `system_config` (`config_key`, `config_value`, `remark`)
SELECT 'user_agreement', JSON_QUOTE(CONCAT(
  '欢迎使用金管家服务。', '\n\n',
  '1. 用户应保证注册信息真实、准确、完整。', '\n',
  '2. 用户应妥善保管账号与密码，并对账号下的操作承担责任。', '\n',
  '3. 平台有权在法律法规允许范围内提供服务支持与运营管理。', '\n',
  '4. 用户使用过程中不得发布违法违规内容，不得干扰平台正常运行。', '\n',
  '5. 如因业务需要调整服务内容，平台将通过合理方式进行通知。'
)), '协议文本'
WHERE NOT EXISTS (
  SELECT 1 FROM `system_config` WHERE `config_key` = 'user_agreement'
);

INSERT INTO `system_config` (`config_key`, `config_value`, `remark`)
SELECT 'privacy_policy', JSON_QUOTE(CONCAT(
  '我们重视您的个人信息与隐私保护。', '\n\n',
  '1. 为提供服务，系统可能收集手机号、姓名、门店、任务记录等必要信息。', '\n',
  '2. 收集的信息仅用于身份识别、业务处理、服务通知与运营保障。', '\n',
  '3. 未经用户授权或法律法规要求，不会向无关第三方泄露个人信息。', '\n',
  '4. 我们将采取合理安全措施保护您的数据，防止丢失、泄露或被非法使用。', '\n',
  '5. 用户可通过平台提供的方式查询、更正相关信息。'
)), '协议文本'
WHERE NOT EXISTS (
  SELECT 1 FROM `system_config` WHERE `config_key` = 'privacy_policy'
);
