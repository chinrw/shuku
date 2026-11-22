package io.github.chinrw.shuku

import android.Manifest
import android.app.Activity
import android.os.Build
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ConnectionArgs {
    var interactive: Boolean = false
}

@TauriPlugin(permissions = [
    Permission(strings = [Manifest.permission.ACCESS_LOCAL_NETWORK], alias = "lan")
])
class LocalNetworkPlugin(activity: Activity) : Plugin(activity) {
    @Command
    fun afterFailure(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < 37 || getPermissionState("lan") == PermissionState.GRANTED) {
            invoke.resolve(JSObject().apply { put("retry", false) })
        } else if (invoke.parseArgs(ConnectionArgs::class.java).interactive) {
            requestPermissionForAlias("lan", invoke, "afterPermission")
        } else {
            // 最近使用和列表在线状态都是后台探测，不能各自弹一次系统权限框。
            invoke.reject("连接失败；如果是局域网书库，请点服务器或“保存并打开”以授权。")
        }
    }

    @PermissionCallback
    fun afterPermission(invoke: Invoke) {
        if (getPermissionState("lan") == PermissionState.GRANTED) {
            invoke.resolve(JSObject().apply { put("retry", true) })
        } else {
            invoke.reject("未获得局域网权限。请在系统设置 → 应用 → Shuku → 权限中允许访问附近设备。")
        }
    }
}
