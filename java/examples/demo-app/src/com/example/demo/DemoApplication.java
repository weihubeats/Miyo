package com.example.demo;

import io.miyo.debug.InstanceProvider;
import io.miyo.debug.MiyoDebugRuntime;
import io.miyo.debug.RuntimeConfig;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * 演示应用：把 Miyo Java Debug Runtime 嵌入一个「订单 + 支付」业务。
 *
 * 启动：MIYO_DEBUG_TOKEN=<token> java -cp <sdk-classes>:<demo-classes>
 *       com.example.demo.DemoApplication
 *
 * 显式注册白名单（OrderService / PaymentService）后，Miyo 桌面端即可
 * 连接 http://127.0.0.1:9918 发现并调用这些方法。未注册的类一律拒绝。
 */
public final class DemoApplication {

    // ---- 业务模型（模拟真实项目 DTO）----

    public static final class CreateOrderRequest {
        public String userId;
        public List<Item> items = new ArrayList<>();
        public String coupon;

        public static final class Item {
            public String sku;
            public long qty;
        }
    }

    public static final class OrderResult {
        private final String orderNo;
        private final double amount;
        private final String status;

        OrderResult(String orderNo, double amount, String status) {
            this.orderNo = orderNo;
            this.amount = amount;
            this.status = status;
        }

        public String getOrderNo() {
            return orderNo;
        }

        public double getAmount() {
            return amount;
        }

        public String getStatus() {
            return status;
        }
    }

    // ---- 业务服务 ----

    public static class OrderService {
        private final Map<String, OrderResult> orders = new LinkedHashMap<>();

        public OrderResult createOrder(String userId, CreateOrderRequest request) {
            double amount = request.items.stream().mapToDouble(i -> i.qty * 9.9).sum();
            OrderResult order = new OrderResult("NO-" + UUID.randomUUID(), amount, "CREATED");
            orders.put(order.getOrderNo(), order);
            return order;
        }

        public OrderResult find(String orderNo) {
            OrderResult order = orders.get(orderNo);
            if (order == null) {
                throw new IllegalArgumentException("订单不存在：" + orderNo);
            }
            return order;
        }

        public static double calcTotal(List<Long> quantities) {
            return quantities.stream().mapToLong(q -> q).sum() * 9.9;
        }
    }

    public static class PaymentService {
        public String previewPayment(String orderNo) {
            return "PREVIEW:" + orderNo + ":9.9";
        }

        public String pay(String orderNo, double amount) {
            if (amount <= 0) {
                throw new IllegalStateException("支付金额必须为正：" + amount);
            }
            return "PAID:" + orderNo + ":" + amount;
        }
    }

    public static class MathUtil {
        public static long add(long a, long b) {
            return a + b;
        }
    }

    // ---- 入口 ----

    public static void main(String[] args) throws Exception {
        String token = System.getenv("MIYO_DEBUG_TOKEN");
        if (token == null || token.trim().isEmpty()) {
            System.err.println("缺少环境变量 MIYO_DEBUG_TOKEN（Runtime 禁止无鉴权启动）");
            System.exit(2);
        }
        int port = Integer.parseInt(System.getenv().getOrDefault("MIYO_DEBUG_PORT", "9918"));

        OrderService orderService = new OrderService();
        PaymentService paymentService = new PaymentService();

        MiyoDebugRuntime runtime = MiyoDebugRuntime.create(
                        RuntimeConfig.builder(token.trim())
                                .appName("demo-order-app")
                                .port(port))
                .register("orderService", OrderService.class, InstanceProvider.of(orderService))
                .register("paymentService", PaymentService.class, InstanceProvider.of(paymentService))
                .register(null, MathUtil.class, InstanceProvider.staticInstance())
                .start();

        System.out.println("已启动 " + runtime.describe());
        System.out.println("白名单：OrderService / PaymentService / MathUtil");
        Runtime.getRuntime().addShutdownHook(new Thread(runtime::stop));
        Thread.currentThread().join();
    }

    private DemoApplication() {
    }
}
