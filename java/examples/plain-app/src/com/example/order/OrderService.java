package com.example.order;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ThreadLocalRandom;

/**
 * 订单服务（实例方法 + DTO / 枚举 / Map / 异常 / 慢方法）。
 * Attach 调试验证的实例目标：调用需经 Bean / 新建实例两种解析方式。
 */
public class OrderService {

    private final Map<Long, Order> orders = new LinkedHashMap<>();
    private long nextId = 1000;

    /** 创建订单（实例方法；嵌套 DTO + 枚举参数）。 */
    public Order createOrder(CreateOrderRequest request) {
        if (request.getItems() == null || request.getItems().isEmpty()) {
            throw new IllegalArgumentException("订单至少需要一个商品");
        }
        if (request.getChannel() == Channel.WHOLESALE && request.getTotal() < 1000) {
            throw new IllegalStateException("批发渠道单笔最低 1000 元");
        }
        Order order = new Order();
        order.setId(nextId);
        order.setOrderNo("SO-" + nextId++);
        order.setCustomerName(request.getCustomerName());
        order.setChannel(request.getChannel());
        order.setItems(new ArrayList<>(request.getItems()));
        long total = 0;
        for (OrderItem item : request.getItems()) {
            total += item.getUnitPrice() * item.getQuantity();
        }
        order.setTotal(total);
        order.setStatus(OrderStatus.CREATED);
        synchronized (orders) {
            orders.put(order.getId(), order);
        }
        return order;
    }

    /** 查询订单。 */
    public Order getOrder(String orderNo) {
        for (Order order : orders.values()) {
            if (order.getOrderNo().equals(orderNo)) {
                return order;
            }
        }
        throw new IllegalArgumentException("订单不存在：" + orderNo);
    }

    /** 统计（Map 返回值）。 */
    public Map<String, Object> stats() {
        Map<String, Object> out = new LinkedHashMap<>();
        synchronized (orders) {
            out.put("order_count", orders.size());
            long sum = 0;
            for (Order order : orders.values()) {
                sum += order.getTotal();
            }
            out.put("total_amount", sum);
        }
        out.put("next_id", nextId);
        return out;
    }

    /** 慢方法（超时 / 取消语义验证用）。 */
    public String slowOperation(long seconds) throws InterruptedException {
        long deadline = System.currentTimeMillis() + seconds * 1000;
        while (System.currentTimeMillis() < deadline) {
            Thread.sleep(50);
        }
        return "done after " + seconds + "s";
    }

    /** 随机失败（异常链验证用）。 */
    public Order payOrder(String orderNo) {
        Order order = getOrder(orderNo);
        if (ThreadLocalRandom.current().nextInt(10) < 3) {
            throw new IllegalStateException("支付网关暂时不可用（模拟随机失败）");
        }
        order.setStatus(OrderStatus.PAID);
        return order;
    }

    public int orderCount() {
        return orders.size();
    }
}
