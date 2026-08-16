package top.yunjian.mobile

import android.content.Context

object YunjianAndroid {
    @Volatile
    private var initialized = false

    @JvmStatic
    fun initialize(context: Context) {
        if (initialized) return
        synchronized(this) {
            if (initialized) return
            System.loadLibrary("yunjian_mobile")
            initializeNative(context.applicationContext)
            initialized = true
        }
    }

    @JvmStatic
    private external fun initializeNative(context: Context)
}
