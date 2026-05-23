# 实验 3：运行时字节码注入 — Rust 注入 + JVM Agent

用 Rust 实现 JVM Attach Protocol，向运行中的 Spring Boot 应用注入 Java Agent，在内存中修改 `login()` 方法字节码，绕过密码验证。

## 架构

```
┌──────────────────┐         JVM Attach Protocol          ┌──────────────────────┐
│  rust-injector   │ ─── UNIX socket (.java_pid{PID}) ──→ │  Spring Boot (JVM)   │
│  (macOS libproc  │         load instrument               │                      │
│   + Attach API)  │ ─── agent JAR path ─────────────────→ │  InjectAgent.java    │
│                  │                                        │  ASM 字节码改写      │
└──────────────────┘                                        └──────────────────────┘
```

三个模块：

| 模块 | 语言 | 职责 |
|------|------|------|
| `java-app/` | Java / Spring Boot | 登录系统：MD5 密码校验 + Token 颁发 + Thymeleaf 前端 |
| `java-agent/` | Java / ASM | 运行时修改 `AuthService.login()` 字节码，跳过密码校验 |
| `rust-injector/` | Rust | macOS 进程扫描 + JVM Attach Protocol 实现，将 agent 注入目标 JVM |

## 运行

```bash
# 1. 构建 Java Agent (fat JAR，内含 ASM)
cd examples/03-bytecode-inject/java-agent
mvn clean package -q

# 2. 构建 Spring Boot 应用
cd ../java-app
mvn clean package -q -DskipTests

# 3. 构建 Rust 注入器
cd ../..
cargo build -p rust-injector

# 终端 1: 启动 Spring Boot
cd examples/03-bytecode-inject/java-app
java -jar target/auth-app-1.0.0.jar

# 终端 2: 运行注入器
./target/debug/rust-injector

# 验证
curl -X POST http://localhost:8080/api/login -d 'username=admin&password=anything'
# → {"success":true,"token":"xxx","message":"login success"}

# 备选方案（如果动态 Attach 失败）
java -javaagent:../java-agent/target/inject-agent-1.0.0.jar -jar target/auth-app-1.0.0.jar
```

## 踩过的坑

### 坑 1：macOS 的 TMPDIR 不是 `/tmp/`

JVM Attach Protocol 的 UNIX socket 文件在 `$TMPDIR/.java_pid{PID}`。macOS 上 `TMPDIR` 是 `/var/folders/xx/xxx/T/`，不是 `/tmp/`。只搜 `/tmp/` 永远找不到 socket。

**修复**: 用 `std::env::var("TMPDIR")` 获取真实临时目录，同时在 `/tmp`、`/private/tmp`、`TMPDIR` 三个位置都创建 `.attach_pid` 触发文件。

### 坑 2：动态加载 Agent 时 ASM 类找不到

`NoClassDefFoundError: org/objectweb/asm/ClassVisitor` — 通过 Attach API 动态加载的 agent JAR 只能访问自身 JAR 内的类，外部依赖（ASM）不在 classpath 上。

**修复**: 用 `maven-shade-plugin` 把 ASM 打进 agent JAR（fat JAR）：

```xml
<plugin>
    <groupId>org.apache.maven.plugins</groupId>
    <artifactId>maven-shade-plugin</artifactId>
    <configuration>
        <minimizeJar>true</minimizeJar>
    </configuration>
</plugin>
```

### 坑 3：ASM MethodVisitor 的 visitMaxs/visitEnd 时机

在 `visitCode()` 内部调用 `mv.visitMaxs()` 和 `mv.visitEnd()` 会导致 ClassWriter 状态混乱，生成的字节码无效。同时把 `visitMaxs()` 和 `visitEnd()` 覆盖为 no-op 会阻止 COMPUTE_MAXS 重新计算栈深度。

**修复**:
- `visitCode()` 中只写新指令，不调用 `visitMaxs` / `visitEnd`
- `visitMaxs` 和 `visitEnd` 不覆盖，透传给 ClassWriter
- 使用 `COMPUTE_MAXS | COMPUTE_FRAMES` 让 ASM 自动重算

### 坑 4：`/api/login` 被自己的认证拦截器拦截了

`AuthInterceptor` 配置了 `addPathPatterns("/api/**")`，登录接口本身也在拦截范围内，导致登录请求返回 401。

**修复**: `excludePathPatterns("/api/login")`。

## 涉及的 macOS / JVM API

### Rust 端 — macOS libproc

| 函数 | 作用 |
|------|------|
| `proc_listpids()` | 枚举所有进程 PID |
| `proc_name()` | 获取进程名 |
| `proc_pidinfo(PROC_PIDPATHINFO)` | 获取进程可执行文件路径 |
| `libc::kill(pid, SIGQUIT)` | 向 JVM 发送信号触发 AttachListener |

### JVM Attach Protocol

```
客户端                                         JVM
──────                                        ────
1. 创建 .attach_pid{PID} 文件在 cwd 和 TMPDIR
2. kill(pid, SIGQUIT)                  →      检测 .attach_pid 文件
                                               启动 AttachListener 线程
                                               创建 UNIX socket: $TMPDIR/.java_pid{PID}
3. connect($TMPDIR/.java_pid{PID})     ←
4. send("1\0load\0instrument\0false\0  →      加载 agent JAR
        {agent_path}\0")                        调用 agentmain()
5. recv("0\nreturn code: 0")           ←      返回结果
```

协议格式: `<version>\0<command>\0<arg1>\0<arg2>\0...`，所有字段以 `\0` 分隔。

### Java Agent — ASM 字节码操作

| ASM 类 | 作用 |
|--------|------|
| `ClassReader` | 读取现有 .class 字节码 |
| `ClassWriter` | 生成新的 .class 字节码 |
| `ClassVisitor` | 访问/修改类结构 |
| `MethodVisitor` | 访问/修改方法体 |

`LoginMethodPatcher` 的策略：在 `visitCode()` 中写入全新的指令序列，把原始方法的所有指令覆盖为空操作（no-op）：

```
原始:  MD5(password) → 比较 → 条件跳转 → 成功/失败
修改后: this.tokenService.createToken(username) → LoginResult.ok(token) → return
```

## 运行结果

```
$ java -jar target/auth-app-1.0.0.jar

$ ./target/debug/rust-injector

 ╔══════════════════════════════════════════════════════════╗
 ║               Rust Bytecode Injector v1.0               ║
 ╚══════════════════════════════════════════════════════════╝

[*] TMPDIR: /var/folders/w3/.../T/
Found 1 Java process(es):
  [1] PID:  40675 | java

[+] Found attach socket: /var/folders/w3/.../T/.java_pid40675
[+] Agent load response: 0
return code: 0

 ╔══════════════════════════════════════════════════════════╗
 ║                  INJECTION SUCCESSFUL                    ║
 ╚══════════════════════════════════════════════════════════╝

# 注入前
$ curl -X POST http://localhost:8080/api/login -d 'username=admin&password=wrong'
{"success":false,"token":null,"message":"invalid username or password"}

# 注入后
$ curl -X POST http://localhost:8080/api/login -d 'username=admin&password=anything'
{"success":true,"token":"YWRtaW46MTcxN...","message":"login success"}
```
