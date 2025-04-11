# Blocking waiting for file lock on package cache 卡顿

删除锁定的文件即可  请在powershell 执行
```
# 删除用户级锁文件
Remove-Item -Path "$env:USERPROFILE\.cargo\.package-cache" -Force

# 若使用全局安装，删除系统级锁文件（谨慎操作！）
Remove-Item -Path "C:\Program Files\Rust\.cargo\.package-cache" -Force```