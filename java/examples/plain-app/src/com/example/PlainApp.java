package com.example;

import com.example.order.Channel;
import com.example.order.CreateOrderRequest;
import com.example.order.Order;
import com.example.order.OrderItem;
import com.example.order.OrderService;

import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/**
 * 常驻演示应用（零 Miyo 依赖）：启动后每 10 秒向 stdout 打点，供
 * Miyo Java Debug 2.0 的「本地 JVM 自动发现 + Attach」验收使用。
 *
 * 启动方式：java -cp build com.example.PlainApp [--traffic]
 * --traffic：周期性访问 Spring-like 入口（无 Spring 时仅打点）。
 */
public final class PlainApp {

    private static final OrderService ORDER_SERVICE = new OrderService();

    public static void main(String[] args) throws Exception {
        String mxName = java.lang.management.ManagementFactory.getRuntimeMXBean().getName();
        String pid = mxName.contains("@") ? mxName.substring(0, mxName.indexOf('@')) : mxName;
        System.out.println("[plain-app] started pid=" + pid);
        // 触发 Calculator 类加载（调试协议只能看到「已加载」的类——演示该边界）
        System.out.println("[plain-app] warmup add=" + com.example.calculator.Calculator.add(1, 2));
        // 预置一条订单，attach 后可直接 getOrder / stats 验证
        CreateOrderRequest request = new CreateOrderRequest();
        request.setCustomerName("预置客户");
        request.setChannel(Channel.ONLINE);
        OrderItem item = new OrderItem();
        item.setSku("SKU-001");
        item.setProductName("演示商品");
        item.setUnitPrice(19900);
        item.setQuantity(2);
        request.setItems(java.util.Collections.singletonList(item));
        Order order = ORDER_SERVICE.createOrder(request);
        System.out.println("[plain-app] seeded order " + order.getOrderNo());

        ScheduledExecutorService ticker = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread t = new Thread(r, "plain-app-ticker");
            t.setDaemon(true);
            return t;
        });
        ticker.scheduleAtFixedRate(() -> {
            System.out.println("[plain-app] alive orders=" + ORDER_SERVICE.orderCount());
        }, 10, 10, TimeUnit.SECONDS);

        // 主线程常驻（attach 目标必须存活）
        Thread.currentThread().join();
    }
}
