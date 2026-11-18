package com.example.web;

import lombok.Data;

/**
 * 用户
 */
@Data
public class UserDto {

    /** 用户名 */
    @NotNull
    private String username;

    /** 年龄 */
    private Integer age;

    /** 角色列表 */
    private List<String> roles;

    /** 收货地址 */
    private Address address;

    /** 内部标记 */
    @JsonIgnore
    private String internal;
}