# RustDesk Windows x64 受管设备部署与 API 契约

使用现有的自建 `hbbs/hbbr`。设备端安装后生成独立的永久访问密码，并使用 HTTPS 和 libsodium sealed box 将 RustDesk ID 与密码上报到自定义管理 API。员工在桌面端登录，点击分配给自己的设备即可连接。API 控制凭据发放，RustDesk 被控端校验永久密码。

## GitHub Actions 与版本

在仓库 **Settings → Secrets and variables → Actions → Variables** 添加六个 Repository variables。

| 变量 | 格式及示例 |
| --- | --- |
| `RUSTDESK_ID_SERVER` | `hbbs` 地址和端口，例如 `rd.example.com:21116` |
| `RUSTDESK_RELAY_SERVER` | `hbbr` 地址和端口，例如 `rd.example.com:21117` |
| `RUSTDESK_SERVER_PUBLIC_KEY` | 服务端 `id_ed25519.pub` 的单行内容，标准 Base64 |
| `MANAGEMENT_API_URL` | 自定义管理 API 的 HTTPS 基础地址，例如 `https://management.example.com`。客户端追加 `/v1/…`，也可包含路径前缀 |
| `MANAGEMENT_API_ENCRYPTION_PUBLIC_KEY` | libsodium `crypto_box` 公钥，32 字节原始数据的标准 Base64 |
| `MANAGEMENT_API_ENCRYPTION_KEY_ID` | 对应解密密钥的标识，例如 `device-enrollment-v1` |

构建只上传 EXE 安装程序，Actions Secrets 无需添加。API 解密私钥由服务器端的密钥存储管理。RustDesk 服务端公钥与管理 API 加密公钥分别使用。设备令牌和设备密码由客户端在首次运行服务时生成。

推送到 `managed-windows` 分支时自动构建。工作流进入默认分支后，也可在 **Actions → Managed RustDesk Windows x64 → Run workflow** 手动触发。构建校验 RustDesk 版本 **1.5.3** 和固定基线 `669cb8ac62aa20d3626395877d7dd0357aec7ed2`，同一次运行编译触发时的 Git 提交。安装程序文件名为 `rustdesk-managed-1.5.3-x86_64-install.exe`。双击时请保留 `-install.exe` 后缀，批量部署可以从管理员终端传入 `--silent-install` 参数。安装后的客户端固定使用编译时的服务器配置。受管控制端通过自建服务器按设备 ID 建立连接，仍可使用 NAT 穿透或中继。启动更新检查、自动更新和内置升级入口均关闭。需要升级时由管理员重新打包和部署。

## 安装和注册

1. 在被控机运行 EXE 并完成安装。安装程序启动 RustDesk 服务，服务自动向管理 API 注册。批量部署可从管理员终端运行 `--silent-install`。如果此前安装了 MSI 版，先从 Windows 的「已安装的应用」卸载 MSI 版，再使用 EXE 安装。
2. Windows 服务首次启动时生成设备 UUID 和随机设备令牌，将它们写入本机的受保护目录，然后提交首次注册请求。管理员无需签发令牌或运行脚本。后续请求使用这个设备令牌。
3. Windows 服务约每 30 秒同步一次。它读取本机 RustDesk ID、上传待生效凭据、设置永久密码，再确认激活。失败后自动重试。重启、断网和响应丢失时，复用原来的设备 UUID 和令牌。
4. 本地状态位于 `%ProgramFiles%\RustDesk-Managed-State`。设备令牌只由 SYSTEM 和 Administrators 访问。轮换确认后，状态中保留设备身份、当前凭据版本和 RustDesk ID。
5. 自动注册只建立一条未分配的设备记录。管理员仍需在自己的 API 后台核对并分配设备。安装同一 EXE 的控制端也会注册自己的设备，但员工可以先用管理账号登录并连接已经分配给自己的设备。

控制端在主界面打开 **Managed devices**，登录、查看分配列表并点击处于 `ready` 状态的设备。凭据在当前连接内使用，沿用 RustDesk 已有的共享凭据路径。

## HTTP 接口

设备端和控制端调用以下接口。除注明空请求体的接口外，POST 请求使用 `Content-Type: application/json`。所有地址均相对于 `MANAGEMENT_API_URL`。服务器应使用有效的 HTTPS 证书，直接响应各接口。

| 方法和路径 | 身份 | 请求体 | 成功响应 |
| --- | --- | --- | --- |
| `POST /v1/devices/enroll` | 首次注册，无 `Authorization` 请求头 | `{"device_id":"<UUID>","device_token":"<设备令牌>"}` | `204`，空响应 |
| `GET /v1/devices/{deviceId}/pending-actions` | `Bearer <设备令牌>` | 无 | `200 {"minimum_version":1}` |
| `POST /v1/devices/{deviceId}/credentials` | `Bearer <设备令牌>` | `{"key_id":"device-enrollment-v1","ciphertext":"<Base64>"}` | `204`，空响应 |
| `POST /v1/devices/{deviceId}/credentials/{version}/activate` | `Bearer <设备令牌>` | `{"operation_id":"<UUID>"}` | `204`，空响应 |
| `POST /v1/auth/login` | 用户名和密码 | `{"username":"<员工账号>","password":"<员工登录密码>"}` | `200 {"access_token":"<员工令牌>"}` |
| `GET /v1/me/devices` | `Bearer <员工令牌>` | 无 | `200 {"devices":[{"device_id":"<UUID>","name":"<显示名称>","status":"ready"}]}` |
| `POST /v1/devices/{deviceId}/connect` | `Bearer <员工令牌>` | 空请求体 | `200 {"rustdesk_id":"<RustDesk ID>","password":"<设备永久密码>"}` |

服务器管理端还应提供等效的设备分配操作。客户端不调用它们，服务器可以实现 `PUT /v1/devices/{id}/assignments/{userId}` 和 `DELETE /v1/devices/{id}/assignments/{userId}`，或使用自己的管理界面和接口。撤销操作必须原子更新分配关系与轮换要求。

### 注册与身份

- `device_id` 是客户端生成的 UUID。API 使用它作为设备主键和权限关联键。RustDesk ID 用于建立远程连接，可以随凭据版本更新。
- `device_token` 是客户端生成的 32 字节随机值，以无填充的 Base64URL 编码。首次注册时它在 HTTPS 请求体中提交。API 保存其摘要，并将它限定到对应设备的三个后续接口。
- 首次注册时原子创建一条未分配记录。同一 UUID 和同一令牌的重试应返回 `204`，包括首次成功响应丢失的情况。相同 UUID 搭配不同令牌时拒绝且不得覆盖原记录。先前已注册并保存了设备令牌的客户端升级后继续使用原身份。
- 首次注册接口允许任何能访问 API 的客户端提交新设备记录。限流并校验 UUID 和令牌格式。管理员在分配前核对设备，注册本身不授予连接或员工权限。RustDesk ID 仅用于连接，不应用它自动分配设备或替换已有记录。
- 用户登录由自定义 API 负责。建议发放短时员工访问令牌并为登录限制速率。设备令牌和员工令牌的权限分开。服务端对无效身份返回 `401`，对有效身份访问未分配设备返回 `403` 或 `404`。

### 加密、暂存与激活

`ciphertext` 是 libsodium `crypto_box_seal` 输出的标准 Base64 字符串。服务端根据 `key_id` 选择对应的公私钥，Base64 解码后调用 `crypto_box_seal_open`，再解析 UTF-8 JSON。示例中的 UUID 和密码均为占位内容。

```json
{
  "device_id": "2644d1f4-d3f1-4fff-9d03-1d9b76873273",
  "rustdesk_id": "123456789",
  "password": "<base64url-encoded-random-password>",
  "version": 1,
  "operation_id": "0d733803-31f8-4808-930b-e0b716298b31"
}
```

1. 检查设备令牌、URL 路径和密文中的 `device_id` 是否指向同一设备。校验字段、范围为 1 至 4,294,967,295 的整数版本和操作 UUID。RustDesk ID 使用 1 至 64 个 ASCII 大小写字母、数字、下划线或连字符。新密码是 32 字节随机值的无填充 Base64URL 编码。
2. 接收新版本后，保存为待生效状态，并暂停发放此前的密码。对同一设备、版本、操作 UUID 和**相同解密内容**的重试返回成功，sealed box 每次加密的密文本身会不同。同一操作或版本对应不同内容时返回 `409`。同一设备只应有一个最新的待生效版本，较新的有效版本可以取代较旧版本。
3. 激活接口仅接受设备当前已暂存的版本及对应操作 UUID。激活成功后才能发放这一版本。已激活版本的同一操作重复确认应返回成功，重复请求不得回退现行状态或重新打开已被暂停的发放。并发上传、激活和撤权在服务端按设备串行化。
4. `minimum_version` 是允许发放的最低版本，初始为 `1`。激活响应丢失时，API 可以重复确认同一操作。若后台在新版本暂存后又撤权，可提高最低版本，客户端下一轮会跳过已过时的待生效版本。低于最低版本的凭据一律不发放。对旧暂存请求可返回 `409`，客户端会重新轮询。待生效版本和已激活版本的精确重试应在判为旧写入之前识别。
5. 服务端使用独立的密钥管理服务保护解密私钥和数据库中的凭据。审计记录设备、用户、操作、时间和结果。密码、令牌、请求正文、解密后的数据和带凭据的连接响应不进入日志。

### 分配与撤权

- 设备列表仅返回当前用户分配的设备。使用 `ready` 表示凭据可发放，`pending` 和 `rotation_pending` 等状态会在客户端显示为不可连接。列表不含密码。
- 每次连接请求重新以当前员工身份检查设备分配、待生效状态和最低版本。只有通过检查才能返回 RustDesk ID 和永久密码。连接响应应设置 `Cache-Control: no-store`。
- 撤销分配时，在同一事务中撤权，并将最低版本提高到超过所有已知凭据版本的值。暂停发放到新版本确认生效。设备离线期间显示待轮换。在线设备轮询后生成新密码并重新激活，剩余获授权用户可继续连接。
- 对已经发放的永久密码，轮换完成前仍应按可被重复使用处理。密码轮换不会主动断开已有会话。需要每次新会话实时授权时，可另行实施被控端强制核销的一次性会话票据。

## 验收

1. 下载 Actions 生成的 EXE 安装程序，在干净的 Windows x64 测试机上安装。验证连接自建 ID 和 Relay 服务器，检查 RustDesk 版本为 1.5.3，确认关闭内置更新。
2. 安装两台设备，确认服务自动注册成功后分别分配给两名员工。检查同一设备注册和上报的重试不会生成重复记录，两台设备有不同的永久密码。
3. 验证员工能从列表一键连接自己的设备，未登录和越权请求被拒绝。分别测试 P2P 和中继，并在被控机重启后复测。
4. 模拟 API 在设置密码后短暂不可用，再恢复服务，确认客户端重用同一操作，激活前 API 暂停发放。
5. 验证在线和离线撤权。撤权后 API 立即拒绝原用户取凭据，确认设备轮换后旧密码无法建立新会话，且日志中没有明文密码或令牌。
