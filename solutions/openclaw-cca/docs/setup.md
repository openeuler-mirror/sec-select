# 适用范围 (OpenEuler 24.03 SP4 LTS最小安装)
本文假设RBS服务器运行在`192.168.1.1`，且基于以下软件包版本（`0.0.1-30.oe2403`）提供构建教程。
|软件包|大小|更新日期|
|---|---|---|
global-trust-authority-agent-0.0.1-30.oe2403sp4.x86_64.rpm|	20.9 MiB|	2026-Jun-30 15:38
global-trust-authority-cli-0.0.1-30.oe2403sp4.x86_64.rpm|	19.7 MiB|	2026-Jun-30 15:40
global-trust-authority-debuginfo-0.0.1-30.oe2403sp4.x86_64.rpm|	425.4 MiB|	2026-Jun-30 15:41
global-trust-authority-debugsource-0.0.1-30.oe2403sp4.x86_64.rpm|	13.3 MiB|	2026-Jun-30 15:38
global-trust-authority-key-manager-0.0.1-30.oe2403sp4.x86_64.rpm|	3.8 MiB|	2026-Jun-30 15:38
global-trust-authority-server-0.0.1-30.oe2403sp4.x86_64.rpm|	38.2 MiB|	2026-Jun-30 15:40
globaltrustauthority-rbs-cli-0.0.1-2.oe2403sp4.x86_64.rpm|	4.5 MiB|	2026-Jun-30 15:39
globaltrustauthority-rbs-debuginfo-0.0.1-2.oe2403sp4.x86_64.rpm|	182.7 MiB|	2026-Jun-30 15:39
globaltrustauthority-rbs-debugsource-0.0.1-2.oe2403sp4.x86_64.rpm|	10.7 MiB|	2026-Jun-30 15:41
globaltrustauthority-rbs-rbc-devel-0.0.1-2.oe2403sp4.x86_64.rpm|	7.5 MiB|	2026-Jun-30 15:38
globaltrustauthority-rbs-rbs-0.0.1-2.oe2403sp4.x86_64.rpm|	8.5 MiB|	2026-Jun-30 15:40
# RBS服务器环境（非虚机）
## 环境安装
安装主要软件包
```bash
sudo dnf install global-trust-authority-server globaltrustauthority-rbs-rbs
#global-trust-authority-key-manager 可以不需要
```

<!-- 配置openBao
```bash
cat > /etc/yum.repos.d/openbao.repo << EOF
[openbao]
name=openbao
baseurl=https://pkgs.openbao.org/rpm/$basearch
repo_gpgcheck=0
gpgcheck=1
enabled=1
gpgkey=https://openbao.org/assets/openbao-gpg-pub-20240618.asc
sslverify=1
sslcacert=/etc/pki/tls/certs/ca-bundle.crt
metadata_expire=300
EOF

dnf update
``` -->

<!-- 配置key-manager所需参数 `/usr/local/key_manager/bin/.env`
```bash
KEY_MANAGER_ROOT_TOKEN=s.u6O7REadDh4C5RwxCkfIdfOh #Root Token
KEY_MANAGER_SECRET_ADDR=http://127.0.0.1:8200/
```

配置key-manager相关密钥 -->
<!-- 配置GTA-server相关密钥
```bash
mkdir -p /etc/attestation_server/certs
sudo openssl req -x509 -newkey rsa:3072 -out /etc/attestation_server/certs/km_cert.pem -keyout /etc/attestation_server/certs/km_key.pem -noenc -subj "/CN=RootCA-KeyManager"

sudo openssl genrsa -out /etc/attestation_server/certs/key_manager_server_key.pem 3072
# sudo openssl req -new -key /etc/attestation_server/certs/key_manager_server_key.pem -out /etc/attestation_server/certs/key_manager_server.csr #-subj "/CN=KeyManager" -addext "subjectAltName=IP:127.0.0.1"
# sudo openssl x509 -req -in /etc/attestation_server/certs/key_manager_server.csr -CA /etc/attestation_server/certs/km_cert.pem -CAkey /etc/attestation_server/certs/km_key.pem -CAcreateserial -out /etc/attestation_server/certs/key_manager_server_cert.pem -copy_extensions copy

sudo openssl genrsa -out /etc/attestation_server/certs/ra_client_key.pem 3072
sudo openssl req -new -key /etc/attestation_server/certs/ra_client_key.pem -out /etc/attestation_server/certs/ra_client.csr -subj "/CN=RA-Service" #-addext "subjectAltName=IP:127.0.0.1"
sudo openssl x509 -req -in /etc/attestation_server/certs/ra_client.csr -CA /etc/attestation_server/certs/km_cert.pem -CAkey /etc/attestation_server/certs/km_key.pem -CAcreateserial -out /etc/attestation_server/certs/ra_client_cert.pem -copy_extensions copy
``` -->

<!-- 运行key-manager
```bash
/usr/local/key_manager/bin/key_managerd
``` -->

安装GTA-server dependency
```bash
sudo dnf install mysql-server redis cjson
sudo systemctl enable --now mysqld
sudo systemctl enable --now redis
mkdir -p /etc/attestation_server/keys
openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072 -out /etc/attestation_server/keys/fsk_private_key.pem
openssl rsa -in /etc/attestation_server/keys/fsk_private_key.pem -pubout -out /etc/attestation_server/keys/fsk_public_key.pem
openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072 -out /etc/attestation_server/keys/nsk_private_key.pem
openssl rsa -in /etc/attestation_server/keys/nsk_private_key.pem -pubout -out /etc/attestation_server/keys/nsk_public_key.pem
openssl genrsa -out /etc/attestation_server/keys/tsk_private_key.pem 4096
openssl rsa -in /etc/attestation_server/keys/tsk_private_key.pem -pubout -out /etc/attestation_server/keys/tsk_public_key.pem

# 更改MySQL root默认密码和初始化数据库
mysql -u root << EOF
create USER 'ra_user'@'localhost' IDENTIFIED BY 'ra_user_password';
CREATE DATABASE RA;
GRANT ALL PRIVILEGES ON RA.* TO 'ra_user'@'localhost';
FLUSH PRIVILEGES;
EOF
```

<!-- 使用key-manager配置相关密钥
```bash
/usr/local/key_manager/bin/key_manager put --key_name FSK --algorithm rsa_3072 --key_file /etc/attestation_server/keys/fsk_private_key.pem
/usr/local/key_manager/bin/key_manager put --key_name NSK --algorithm rsa_3072 --key_file /etc/attestation_server/keys/nsk_private_key.pem
/usr/local/key_manager/bin/key_manager put --key_name TSK --algorithm rsa_3072 --key_file /etc/attestation_server/keys/tsk_private_key.pem
``` -->

配置`/etc/attestation_server/.env`，关闭HTTPS并设置用户密码
```bash
DB_USER=ra_user
DB_PASSWORD=ra_user_password

HTTPS_SWITCH=0

MYSQL_DATABASE_URL=mysql://ra_user:ra_user_password@127.0.0.1:3306/RA
```

安装RBS dependency
```bash
sudo dnf install sqlite
sqlite3 /var/lib/rbs/rbs.db "SELECT 1;"
```

更改配置`/etc/rbs/rbs.yaml`
```yaml
auth:
  attest_token:
    public_key_path: "/etc/rbs/attest_pub.pem"
    # 注释以下一行
    # jwks_file: "/etc/rbs/attest.jwk"
attestation:
  backends:
    gta:
      rest:
        base_url: "http://127.0.0.1:8080"
        credentials:
          # 注释以下两行
          # main_api_key: "${MAIN_API_KEY}"   # optional (reserved for future use)
          # sub_api_key: "${SUB_API_KEY}"   # optional

```

生成RBS相关密钥
 ```bash
openssl genrsa -out /etc/rbs/admin.pem 4096
openssl rsa -in /etc/rbs/admin.pem -pubout -out /etc/rbs/admin_pub.pem
```
<!--
openssl rsa -pubin -in /etc/attestation_server/keys/tsk_public_key.pem -pubout -out /etc/rbs/attest_pub.pem -traditional
``` -->

```bash
cp /etc/attestation_server/keys/tsk_public_key.pem /etc/rbs/attest_pub.pem
```

生成服务器CA证书
```bash
mkdir -p /etc/attestation_server/certs
sudo openssl req -x509 -newkey rsa:3072 -out /etc/attestation_server/certs/ca.crt -keyout /etc/attestation_server/certs/ca.key -noenc -subj "/CN=GTA"

sudo openssl genrsa -out /etc/attestation_server/certs/server.key 3072
sudo openssl req -new -key /etc/attestation_server/certs/server.key -out /etc/attestation_server/certs/server.csr -subj "/CN=GTA-Server" -addext "subjectAltName=IP:127.0.0.1"
sudo openssl x509 -req -in /etc/attestation_server/certs/server.csr -CA /etc/attestation_server/certs/ca.crt -CAkey /etc/attestation_server/certs/ca.key -CAcreateserial -out /etc/attestation_server/certs/server.crt -copy_extensions copy
```

<!-- 可执行文件位置
```bash
/usr/local/key_manager/bin/key_manager
attestation_service #/usr/bin/attestation_service
rbs #/usr/bin/rbs
``` -->

## RBS Resource存储相关

安装openBao
```bash
# AMD64平台
curl -fsSL -O https://github.com/openbao/openbao/releases/download/v2.6.1/openbao_2.6.1_linux_amd64.rpm
sudo dnf install ./openbao_2.6.1_linux_amd64.rpm

# ARM64平台
curl -fsSL -O https://github.com/openbao/openbao/releases/download/v2.6.1/openbao_2.6.1_linux_arm64.rpm
sudo dnf install ./openbao_2.6.1_linux_arm64.rpm
```

<!-- 配置openBao
```bash
# 警告！！！
# 生产环境中应使用实际X509证书，而非运行以下命令
openssl req -x509 -newkey rsa:4096 -keyout /opt/openbao/tls/tls.key -out /opt/openbao/tls/tls.crt -noenc -subj "/CN=localhost" -addext "subjectAltName=IP:127.0.0.1"
``` -->

配置openBao
```bash
# 使用任意编辑器编辑配置 禁用HTTPS Server 启用HTTP Server（127.0.0.1）
nano /etc/openbao/openbao.hcl
```
编辑后的文件如
```
# Copyright (c) HashiCorp, Inc.
# SPDX-License-Identifier: MPL-2.0

# Full configuration options can be found at https://github.com/openbao/openbao/tree/main/website/content/docs/configuration

ui = true

storage "file" {
  path = "/opt/openbao/data"
}

# HTTP listener
listener "tcp" {
  address = "127.0.0.1:8200"
  tls_disable = 1
}

# HTTPS listener
# listener "tcp" {
#   address       = "0.0.0.0:8200"
#   tls_cert_file = "/opt/openbao/tls/tls.crt"
#   tls_key_file  = "/opt/openbao/tls/tls.key"
# }
# Example AWS KMS auto unseal
#seal "awskms" {
#  region = "us-east-1"
#  kms_key_id = "REPLACE-ME"
#}

```

启动和初始化openBao
```bash
sudo systemctl enable --now openbao
export BAO_ADDR=http://127.0.0.1:8200
bao operator init
```

保存以上命令输出如下
```
Unseal Key 1: 4guvN68/un4SntwLjqyn4OTzoRlNQ7tAgpAcGtESHtzV
Unseal Key 2: 1SMpSQSIIVEBuk1BhGZO3tdg8r+ENtl1vZi5VwAqlQyo
Unseal Key 3: BUwiJEAWSzFXyhRIoyoiHapjwYI5I6JlRVKxn3wMOn2T
Unseal Key 4: u7X+Jc0zkVPQNvwE7Zcyvagi8mW0mKnY6PhSQvwmTUrY
Unseal Key 5: znEXrSzvLha690pmyjL+oh9e2uNhob1zTtvg8mESygs/

Initial Root Token: s.l3J24O59v2NkrQ8fli7u5rEP
```

解密openBao (默认为3次)
```bash
bao operator unseal 4guvN68/un4SntwLjqyn4OTzoRlNQ7tAgpAcGtESHtzV # Unseal key 1
bao operator unseal 1SMpSQSIIVEBuk1BhGZO3tdg8r+ENtl1vZi5VwAqlQyo # Unseal key 2
bao operator unseal BUwiJEAWSzFXyhRIoyoiHapjwYI5I6JlRVKxn3wMOn2T # Unseal key 3
```

配置RBS存储 `/etc/rbs/rbs.yaml`
```yaml
rest:
  listen_addr: "127.0.0.1:6666" #请在实际部署时更改防火墙策略如 "0.0.0.0:6666" 
attestation:
  backends:
    gta:
      rest:
        base_url: "http://127.0.0.1:8080"
resource:
  default_provider: vault
  backends:
    # local:
    #   type: local
    vault:
      type: vault
      url: "http://127.0.0.1:8200"
      token: "s.u6O7REadDh4C5RwxCkfIdfOh" #${VAULT_TOKEN}
      mount_path: "secret"
#     # ca:
#     #   type: ca
#     #   url: "https://ca-server:8443"
#     #   token: "${CA_TOKEN}"
#     #   default_profile: "server"

```

使用Root Token登录openBao
```bash
bao login
```

启动KV存储Secret
```bash
bao secrets enable --path=secret kv-v2
```

## 启动服务
启动服务
```bash
sudo systemctl enable --now attestation_server
sudo systemctl enable --now rbs
```

## 准备安全资源(任意远端cli)
安全资源准备需要使用rbs-cli
```bash
sudo dnf install globaltrustauthority-rbs-cli
```

准备验证脚本
```
cat > rego << EOF
package verification

default attestation_valid = false
attestation_valid {
	input.status == "pass"
}

result = {"policy_matched": attestation_valid}
EOF
```

在RBS Server准备对应的秘密
```bash
# bao kv put secret/<repo>/<res>/<content-type>/<name> 
bao kv put secret/default/secret/mysecret username=private-username

export RBS_SERVER=http://192.168.1.1:6666
export ACCESS_KEY=$(rbs-cli token gen --private-key-file /etc/rbs/admin.pem)
rbs-cli -b ${RBS_SERVER} -t ${ACCESS_KEY} res-policy create --name policy-01 --content @./rego
# 保存上一步生成的policy-id，如c28a6e63-b0b2-4fdd-9832-4d297f28e31e
rbs-cli -b ${RBS_SERVER} -t ${ACCESS_KEY} res create --provider-name vault --repository-name default --resource-type secret --resource-name mysecret --policy-id c28a6e63-b0b2-4fdd-9832-4d297f28e31e
```
# 安全容器内（虚机）环境安装与验证

在安全容器内安装rbc-cli
```bash
dnf install global-trust-authority-agent globaltrustauthority-rbs-rbc-devel
```

编辑`/etc/attestation_agent/agent_config.yaml`，将所有的`ccel_data_path`更改为`boot_log_file_path`
```bash
sed -i 's|ccel_data_path|boot_log_file_path|' /etc/attestation_agent/agent_config.yaml
```
并更改其中
1. server部分为对应HTTP地址（或导入HTTPS CA证书）
2. plugins/enabled除所需的（如CCA）外保持false

使能CCA（以sudo/root身份运行以下命令）
```bash
modprobe tsm
modprobe arm_cca_guest
mount -t configfs none /sys/kernel/config
export report=/sys/kernel/config/tsm/report/report0
mkdir -p $report
dd if=/dev/urandom bs=64 count=1 > $report/inblob
hexdump -C $report/outblob
hexdump -C $report/auxblob
```

测试RBS与Attestation_Server的连接
```bash
export RBS_SERVER=http://192.168.1.1:6666
curl -X GET ${RBS_SERVER}/rbs/v0/challenge
# 应返回json消息 {"nonce": xxx}
```


<!-- 为GTA添加密钥
```bash
curl -X POST -H "User-Id:rbs-service" -H "Content-Type:application/json" -d '{
  "name": "root.crt",
  "type": ["tpm_boot"],
  "content": "-----BEGIN CERTIFICATE-----\nMIIDcTCCAlmgAwIBAgIUPehnCqFI5+DVnQnggmcy/MX/hYIwDQYJKoZIhvcNAQELBQAwRzELMAkGA1UEBhMCQ04xEDAOBgNVBAoMB3Rlc3QgQ0ExDTALBgNVBAsMBHRlc3QxFzAVBgNVBAMMDlRQTSBST09UIENBIFYyMCAXDTI2MDczMTA3MTkwNFoYDzIwNTYwNzIzMDcxOTA0WjBHMQswCQYDVQQGEwJDTjEQMA4GA1UECgwHdGVzdCBDQTENMAsGA1UECwwEdGVzdDEXMBUGA1UEAwwOVFBNIFJPT1QgQ0EgVjIwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCWEBKTbk7jJvkFcrUnlvxcXlapSfV5mOfB+CLmUc/cgY28+3dDrU9T6jENHsBlxeHRk1fh5d4CI9aj9aD40uYvIPnY3hxsDU4K8frUW5suOTSi9diLgfGmMudXDo1VGdN6DVX/tst+3QwpjRPZ6TTHhAx5OFm/OhcDytw5We2FmTbQqeb689ahBYT5dnZyZDMBtTF2hpVXtWfJ6jt9xx4T6YP+HgbgOEA9zBGorFlYeH0GXgLaujY2DmduT4DkJQ7r9Po4qB+C/AaoPU64g7F9yjUxAgiHTKEONkPjXRA5sVgpPV+WMjfkaw40M4ShYo7kt3cfOwuJmxIT3LVhl3ufAgMBAAGjUzBRMB0GA1UdDgQWBBR6Sq8NTTun+ECKs+A41hIezki7rTAfBgNVHSMEGDAWgBR6Sq8NTTun+ECKs+A41hIezki7rTAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQBSCU4wfpPhIAd2FFSORT4WljOSv9UGmw4axO5nHqKHaOaTeXwGrTrLCdbYDLMjGKPo2kEmyrnmtvRhzLjXTuZmgCxooBJNkOLUN7Ct7/VzUxPWJyfqq6H1n1W0SO2OeRsz4Ts1KdC3cnIEWGHi7FPBrWvxWZ5CncIWXKSMs79xGvIoPQDDuooRJDw2XkJfISRblEYbxRA+CLS9BHD8ogORi1EXif2GTa6m2hFekzZjAdFe0qOWHdIvv8R8g0mvMgUMiiCGwlxwTAJoVFbZu3aw9E7Borwi0d40WsZG766kVPeWiK4ZUmIolERkLEmtrMO0sbAI7EurAWYp0Q9bb7qG\n-----END CERTIFICATE-----"
}' http://127.0.0.1:8080/global-trust-authority/service/v1/cert
``` -->

准备attester密钥
```bash
openssl genrsa -out attester.key 4096
openssl pkey -in attester.key -pubout -out attester.pub
```

测试RBS接口
```bash
# 准备一次性nonce
rbc-cli -b ${RBS_SERVER} challenge > nonce
# 获取并验证evidence
rbc-cli -b ${RBS_SERVER} collect-evidence --nonce @./nonce --attester-pubkey @./attester.pub > evidence

# 通过evidence远程获取安全资源
rbc-cli -b ${RBS_SERVER} get-resource --uri vault/default/secret/mysecret --evidence @evidence
# or
rbc-cli -b ${RBS_SERVER} get-resource --uri vault/default/secret/mysecret --evidence @evidence  --private-key-file attester.key
```

<!-- # TPM后验证
```bash
dnf install tpm2-tools
```

删除TPM中已有的key
```bash
tpm2_nvundefine 0x150001b
tpm2_evictcontrol -C o -c 0x81010020
```

生成TPM相关密钥和证书链
```bash
tpm2_createek -c ek.handle -G rsa -u ek.pub
tpm2_createak -C ek.handle -c ak.ctx -u ak.pub -n ak.name
tpm2_evictcontrol -C o -c ak.ctx 0x81010020
tpm2_readpublic -c 0x81010020 -o ak.pem -f pem -Q
openssl genrsa -out rootCA.key 2048
openssl req -x509 -new -nodes -key rootCA.key -sha256 -days 10950 -out rootCA.crt -subj "/C=CN/O=test CA/OU=test/CN=TPM ROOT CA V2"

cat > cert.conf << EOF
[req]
distinguished_name=req_distinguished_name
req_extensions=v3_req
prompt=no

[req_distinguished_name]
CN = agent
O = My Organization
C = CN

[v3_req]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = clientAuth, serverAuth
EOF

openssl req -new -key rootCA.key -out temp.csr -config cert.conf
openssl x509 -req -in temp.csr -CA rootCA.crt -CAkey rootCA.key -CAcreateserial -days 365 -out ak.crt -force_pubkey ak.pem
openssl x509 -in ak.crt -inform PEM -out ak.der -outform DER
stat -c %s ak.der
tpm2_nvdefine -C o -s 851 0x150001b -a "ppread|ppwrite|authread|ownerread|ownerwrite"
tpm2_nvwrite -C o 0x150001b -i ak.der
```

为GTA添加根密钥（同`rootCA.crt`）
```bash
curl -X POST -H "User-Id:rbs-service" -H "Content-Type:application/json" -d '{
  "name": "root.crt",
  "type": ["tpm_boot"],
  "content": "-----BEGIN CERTIFICATE-----\nMIIDcTCCAlmgAwIBAgIUPehnCqFI5+DVnQnggmcy/MX/hYIwDQYJKoZIhvcNAQELBQAwRzELMAkGA1UEBhMCQ04xEDAOBgNVBAoMB3Rlc3QgQ0ExDTALBgNVBAsMBHRlc3QxFzAVBgNVBAMMDlRQTSBST09UIENBIFYyMCAXDTI2MDczMTA3MTkwNFoYDzIwNTYwNzIzMDcxOTA0WjBHMQswCQYDVQQGEwJDTjEQMA4GA1UECgwHdGVzdCBDQTENMAsGA1UECwwEdGVzdDEXMBUGA1UEAwwOVFBNIFJPT1QgQ0EgVjIwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCWEBKTbk7jJvkFcrUnlvxcXlapSfV5mOfB+CLmUc/cgY28+3dDrU9T6jENHsBlxeHRk1fh5d4CI9aj9aD40uYvIPnY3hxsDU4K8frUW5suOTSi9diLgfGmMudXDo1VGdN6DVX/tst+3QwpjRPZ6TTHhAx5OFm/OhcDytw5We2FmTbQqeb689ahBYT5dnZyZDMBtTF2hpVXtWfJ6jt9xx4T6YP+HgbgOEA9zBGorFlYeH0GXgLaujY2DmduT4DkJQ7r9Po4qB+C/AaoPU64g7F9yjUxAgiHTKEONkPjXRA5sVgpPV+WMjfkaw40M4ShYo7kt3cfOwuJmxIT3LVhl3ufAgMBAAGjUzBRMB0GA1UdDgQWBBR6Sq8NTTun+ECKs+A41hIezki7rTAfBgNVHSMEGDAWgBR6Sq8NTTun+ECKs+A41hIezki7rTAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQBSCU4wfpPhIAd2FFSORT4WljOSv9UGmw4axO5nHqKHaOaTeXwGrTrLCdbYDLMjGKPo2kEmyrnmtvRhzLjXTuZmgCxooBJNkOLUN7Ct7/VzUxPWJyfqq6H1n1W0SO2OeRsz4Ts1KdC3cnIEWGHi7FPBrWvxWZ5CncIWXKSMs79xGvIoPQDDuooRJDw2XkJfISRblEYbxRA+CLS9BHD8ogORi1EXif2GTa6m2hFekzZjAdFe0qOWHdIvv8R8g0mvMgUMiiCGwlxwTAJoVFbZu3aw9E7Borwi0d40WsZG766kVPeWiK4ZUmIolERkLEmtrMO0sbAI7EurAWYp0Q9bb7qG\n-----END CERTIFICATE-----"
}' http://127.0.0.1:8080/global-trust-authority/service/v1/cert
``` -->