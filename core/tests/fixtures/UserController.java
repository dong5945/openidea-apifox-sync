package com.example.web;

import org.springframework.web.bind.annotation.*;
import org.springframework.web.multipart.MultipartFile;

/**
 * 用户管理接口
 */
@RestController
@RequestMapping("/api/users")
public class UserController {

    /**
     * 分页查询用户
     *
     * @param keyword 搜索关键字
     * @param page    页码
     */
    @GetMapping("/page")
    public Result<PageResult<UserDto>> page(@RequestParam String keyword,
                                            @RequestParam(defaultValue = "1") int page,
                                            @RequestHeader(value = "X-Tenant", required = false) String tenant) {
        return null;
    }

    /**
     * 按 ID 查询用户
     *
     * @param id 用户 ID
     */
    @GetMapping("/{id}")
    public Result<UserDto> detail(@PathVariable Long id) {
        return null;
    }

    /** 新建用户 */
    @PostMapping
    public Result<UserDto> create(@RequestBody UserDto body) {
        return null;
    }

    /**
     * 更新用户
     *
     * @deprecated 改用 PATCH
     */
    @PutMapping("/{id}")
    @Deprecated
    public Result<UserDto> update(@PathVariable Long id, @RequestBody UserDto body) {
        return null;
    }

    /** 删除用户 */
    @DeleteMapping("/{id}")
    public Result<Void> remove(@PathVariable Long id) {
        return null;
    }

    /** 上传头像 */
    @PostMapping("/avatar")
    public Result<String> upload(@RequestParam MultipartFile file) {
        return null;
    }
}