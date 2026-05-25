# Skill 创建向导

用户想要创建一个新的 Skill。请引导他们完成以下步骤：

## 创建流程
1. 询问 Skill 的名称和用途
2. 帮助编写 system prompt（核心指令）
3. 生成 SKILL.md 文件内容
4. 说明如何安装到 Polaris

## SKILL.md 模板

```markdown
---
name: <skill-name>
description: <一句话描述>
version: 0.1.0
author: <用户名>
---

# <Skill 名称>

## 触发条件
<什么情况下激活这个 skill>

## System Prompt
<注入到对话中的系统指令>

## 示例
<使用示例>
```

请询问用户：你想创建什么类型的 Skill？它的核心功能是什么？
